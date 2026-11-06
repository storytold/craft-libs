//! Plate mode through the raster path: each ink's separation drawn as a gray film.
//! Adapted from PdfCraft #2770 by Guillem Hermida (`separations::tests` in pdfcraft-render at
//! 6fcf2a4aca29e943e28cb69017ff8f87d00656c6, MIT OR Apache-2.0).
#![cfg(feature = "raster")]

#[path = "support/synthetic_icc.rs"]
mod synthetic_icc;

use craft_pdf::interpret::InterpreterSettings;
use craft_pdf::interpret::color::{CmykConversion, CmykSimulation, Ink, InkSinkFn, NoSimulation, PCS_WHITE, Plate};
use craft_pdf::{
    open_slice,
    raster::{self, Options, RenderCache},
};
use std::sync::{Arc, Mutex};

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

fn render(pdf: &[u8], settings: &InterpreterSettings) -> Vec<u8> {
    render_each(pdf, std::slice::from_ref(settings)).remove(0)
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

/// A spot color whose alternate is a yellowish CMYK.
const GOLD: &str = "[/Separation /Gold /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0.3 1 0] /N 1 >>]";

fn gold_resource() -> String {
    format!("/ColorSpace << /CS0 {GOLD} >>")
}

const CGATS: &[u8] = include_bytes!("../../../vendor/hayro-interpret/assets/CGATS001Compat-v2-micro.icc");

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn device_cmyk_puts_each_component_on_its_own_plate() {
    let s = plates(&page_pdf("0.2 0.4 0.6 0.8 k 0 0 40 40 re f", "", &[]), &inks(&[]), true, None);
    assert_eq!(s.at(20, 20), [20, 40, 60, 80]);
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
fn op_sets_stroke_overprint_and_fill_overprint_too_without_op() {
    let resources = format!("{} /ExtGState << /GS1 << /OP true >> /GS2 << /OP true /op false >> >>", gold_resource());
    let fill = |gs: &str| page_pdf(&format!("0 0 0 1 k 0 0 40 40 re f /{gs} gs /CS0 cs 1 scn 10 10 20 20 re f"), &resources, &[]);
    assert_eq!(separate(&fill("GS1")).at(20, 20)[3], 100, "OP alone also sets op");
    assert_eq!(separate(&fill("GS2")).at(20, 20)[3], 0, "op false keeps fills knocking out");
    let stroke = page_pdf("0 0 0 1 k 0 0 40 40 re f /GS2 gs /CS0 CS 1 SCN 8 w 0 20 m 40 20 l S", &resources, &[]);
    assert_eq!(separate(&stroke).at(20, 20), [0, 0, 0, 100, 100], "strokes overprint with OP");
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

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn gray_and_rgb_are_process_color_and_never_take_the_nonzero_overprint_mode() {
    let process = inks(&[]);
    let gray = page_pdf("1 0 0 0 k 0 0 40 40 re f /M1 gs 0.25 g 10 10 20 20 re f", "/ExtGState << /M1 << /OP true /OPM 1 >> >>", &[]);
    assert_eq!(plates(&gray, &process, true, None).at(20, 20), [0, 0, 0, 75], "gray is black only, and knocks out cyan");
    let s = plates(&page_pdf("1 0 0 rg 0 0 20 40 re f 0.2 0.2 0.2 rg 20 0 20 40 re f", "", &[]), &process, true, None);
    assert_eq!(s.at(10, 20), [0, 100, 100, 0], "red");
    assert_eq!(s.at(30, 20), [0, 0, 0, 80], "a neutral gray goes to black (10.4.2.4)");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn devicen_components_go_to_the_plates_they_name() {
    let resources = "/ColorSpace << /CS0 [/DeviceN [/Cyan /Silver /None] /DeviceCMYK 5 0 R] >>";
    let function = stream("/FunctionType 4 /Domain [0 1 0 1 0 1] /Range [0 1 0 1 0 1 0 1]", b"{ pop 0.5 mul 0 0 }");
    let s = plates(&page_pdf("/CS0 cs 0.3 0.7 1 scn 0 0 40 40 re f", resources, &[function]), &inks(&["Silver"]), true, None);
    assert_eq!(s.at(20, 20), [30, 0, 0, 0, 70]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_all_colorant_paints_every_plate_and_none_paints_nothing() {
    let gray = "/DeviceGray << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >>";
    let resources = format!("/ColorSpace << /CS0 {GOLD} /Reg [/Separation /All {gray}] /No [/Separation /None {gray}] >>");
    let content = "/CS0 cs 0.2 scn 0 0 40 40 re f /Reg cs 1 scn 0 0 20 40 re f /No cs 1 scn 20 0 20 40 re f";
    let s = separate(&page_pdf(content, &resources, &[]));
    assert_eq!(s.at(10, 20), [100, 100, 100, 100, 100], "registration color");
    assert_eq!(s.at(30, 20), [0, 0, 0, 0, 20], "None leaves every plate as it is");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn icc_based_cmyk_is_separated_as_its_components() {
    let pdf = page_pdf("/CS0 cs 0.1 0.3 0.5 0.7 scn 0 0 40 40 re f", "/ColorSpace << /CS0 [/ICCBased 5 0 R] >>", &[stream("/N 4", CGATS)]);
    assert_eq!(plates(&pdf, &inks(&[]), true, None).at(20, 20), [10, 30, 50, 70]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn images_are_separated_sample_by_sample() {
    let cmyk = stream("/Type /XObject /Subtype /Image /Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceCMYK", &[255, 0, 0, 0, 0, 0, 0, 128]);
    let s = plates(&page_pdf("q 40 0 0 40 0 0 cm /Im1 Do Q", "/XObject << /Im1 5 0 R >>", &[cmyk]), &inks(&[]), true, None);
    assert_eq!(s.at(10, 20), [100, 0, 0, 0]);
    assert_eq!(s.at(30, 20), [0, 0, 0, 50]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_spot_color_image_overprints_like_any_paint() {
    let image = stream(&format!("/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace {GOLD}"), &[128]);
    let resources = "/XObject << /Im1 5 0 R >> /ExtGState << /GS1 << /op true >> >>";
    let pdf = |gs: &str| page_pdf(&format!("0 0 0 1 k 0 0 40 40 re f {gs} q 20 0 0 20 10 10 cm /Im1 Do Q"), resources, std::slice::from_ref(&image));
    assert_eq!(separate(&pdf("/GS1 gs")).at(20, 20), [0, 0, 0, 100, 50]);
    assert_eq!(separate(&pdf("")).at(20, 20), [0, 0, 0, 0, 50]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn shadings_are_separated_through_their_color_space() {
    let shading = format!(
        "<< /ShadingType 2 /ColorSpace {GOLD} /Coords [0 0 40 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> /Extend [true true] >>"
    );
    let s = separate(&page_pdf("0 0 0 1 k 0 0 40 40 re f /Sh1 sh", "/Shading << /Sh1 5 0 R >>", &[shading.into_bytes()]));
    let (left, right) = (s.at(4, 20), s.at(36, 20));
    assert!(left[4] < 20 && right[4] > 80, "a gradient on the spot plate: {left:?} {right:?}");
    assert_eq!((left[3], right[3]), (0, 0), "the shading knocks out the black under it");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_uncolored_pattern_paints_the_film_of_the_color_it_is_used_with() {
    let cell = stream("/PatternType 1 /PaintType 2 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >>", b"0 0 10 10 re f");
    let resources = format!("/ColorSpace << /PCS [/Pattern {GOLD}] >> /Pattern << /P1 5 0 R >>");
    let s = separate(&page_pdf("0 0 0 1 k 0 0 40 40 re f /PCS cs 0.5 /P1 scn 0 0 40 40 re f", &resources, &[cell]));
    assert_eq!(s.at(20, 20), [0, 0, 0, 0, 50], "the pattern's spot color, knocking out black");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn soft_masks_keep_the_luminosity_of_their_own_colors() {
    // A mask drawn in blue lets about a tenth of the black through. Drawn as the black
    // plate's film, blue (no black ink) would read as white and let all of it through.
    let group = stream("/Type /XObject /Subtype /Form /BBox [0 0 40 40] /Group << /S /Transparency /CS /DeviceRGB >>", b"0 0 1 rg 0 0 40 40 re f");
    let resources = "/ExtGState << /SM << /SMask << /S /Luminosity /G 5 0 R >> >> >>";
    let s = plates(&page_pdf("/SM gs 0 0 0 1 k 0 0 40 40 re f", resources, &[group]), &[Ink::Black], true, None);
    let black = s.at(20, 20)[0];
    assert!((1..30).contains(&black), "black through a blue luminosity mask: {black}%");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_device_cmyk_luminosity_mask_reads_its_inks_by_the_device_formula() {
    // Black ink through a mask whose DeviceCMYK group paints the left half, at (10, 20), and
    // leaves the backdrop on the right, at (30, 20). Luminosity is 1 - min(1, 0.3c + 0.59m +
    // 0.11y + k) (ISO 32000-2 §10.4.2.2); Gold's alternate is C0 M30 Y100 K0.
    let masked = |paint: &str, backdrop: &str| {
        let group = stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 40 40] /Group << /S /Transparency /CS /DeviceCMYK >> /Resources << {} >>",
                gold_resource()
            ),
            format!("{paint} 0 0 20 40 re f").as_bytes(),
        );
        let resources = format!("/ExtGState << /SM << /SMask << /S /Luminosity /G 5 0 R {backdrop} >> >> >>");
        let s = plates(&page_pdf("/SM gs 0 0 0 1 k 0 0 40 40 re f", &resources, &[group]), &[Ink::Black], true, None);
        [s.at(10, 20)[0], s.at(30, 20)[0]]
    };
    assert_eq!(masked("0 0 0 1 k", ""), [0, 0], "K 100% and the default black backdrop hide all of it");
    assert_eq!(masked("0 0 0 0 k", "/BC [0 0 0 0]"), [100, 100], "paper shows all of it");
    assert_eq!(masked("0.5 0 0 0 k", "/BC [0 0 0 0.25]"), [85, 75]);
    assert_eq!(masked("/CS0 cs 1 scn", ""), [71, 0], "a spot ink reads as its alternate CMYK");
    assert_eq!(masked("1 0 0 rg", ""), [30, 0], "RGB red is C0 M100 Y100 K0 by the device formulas");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_ink_sink_reports_spot_inks_of_patterns_and_images_but_not_of_soft_masks() {
    let image = stream(&format!("/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace {GOLD}"), &[255]);
    let cell = stream(
        "/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << /ColorSpace << /S [/Separation /Silver /DeviceGray << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0.5] /N 1 >>] >> >>",
        b"/S cs 1 scn 0 0 5 5 re f",
    );
    let mask_group = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 40 40] /Group << /S /Transparency /CS /DeviceGray >> /Resources << /ColorSpace << /M [/Separation /MaskOnly /DeviceGray << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >>] >> >>",
        b"/M cs 0.5 scn 0 0 40 40 re f",
    );
    let resources = "/XObject << /Im1 5 0 R >> /Pattern << /P1 6 0 R >> /ExtGState << /SM << /SMask << /S /Luminosity /G 7 0 R >> >> >>";
    let content = "/Pattern cs /P1 scn 0 0 20 20 re f q 10 0 0 10 25 25 cm /Im1 Do Q /SM gs 0 g 0 0 40 40 re f";
    // Each spot ink with its appearance at 100%.
    type Seen = Vec<(Ink, [u8; 3])>;
    let seen: Arc<Mutex<Seen>> = Arc::default();
    let log = seen.clone();
    let sink: InkSinkFn = Arc::new(move |ink: &Ink, appearance: &dyn Fn(f32) -> [u8; 3]| {
        let mut log = log.lock().unwrap();
        if !log.iter().any(|(seen, _)| seen == ink) {
            log.push((ink.clone(), appearance(1.0)));
        }
    });
    render(&page_pdf(content, resources, &[image, cell, mask_group]), &InterpreterSettings { ink_sink: Some(sink), ..Default::default() });
    let seen = seen.lock().unwrap();
    let names: Vec<&str> = seen.iter().map(|(ink, _)| ink.name()).collect();
    assert_eq!(names, ["Silver", "Gold"], "in the order first painted; a mask's ink doesn't print");
    assert_eq!(seen[0].1, [128, 128, 128], "Silver at 100% is its alternate color");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn malformed_color_spaces_and_overprint_parameters_draw_what_they_can() {
    let resources = "/ColorSpace << /A [/DeviceN [] /DeviceGray 0] /B [/Separation] /C [/Indexed /DeviceCMYK 300 <00>] >> /ExtGState << /G << /OP 7 /OPM /X /op (y) >> >>";
    let pdf = page_pdf("/G gs /A cs 1 scn 0 0 9 9 re f /B cs 1 scn /C cs 255 scn 0 0 40 40 re f", resources, &[]);
    plates(&pdf, &inks(&["Gold"]), true, None);
}

// Separating through an output intent's CMYK profile (vendored patch 36).

const PRESS: &str = "PdfCraft synthetic press";

fn press_simulation() -> CmykSimulation {
    CmykSimulation::new(&synthetic_icc::cmyk_profile(PRESS, true)).unwrap()
}

/// What the synthetic press prints for CIE L*a*b*, in whole percent.
fn press(l: f64, a: f64, b: f64) -> Vec<u32> {
    synthetic_icc::btoa(l, a, b).iter().map(|v| (v * 100.0).round() as u32).collect()
}

/// Whether every ink is within `tolerance` percent of `expected`.
fn near(got: &[u32], expected: &[u32], tolerance: u32) -> bool {
    got.len() == expected.len() && got.iter().zip(expected).all(|(g, e)| g.abs_diff(*e) <= tolerance)
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn rgb_separates_through_the_simulated_cmyk_profile() {
    let simulation = press_simulation();
    assert_eq!(simulation.description(), Some(PRESS));
    assert!(!simulation.perceptual());
    let pdf = page_pdf("0.5 0.5 0.5 rg 0 0 40 40 re f", "", &[]);
    // The press prints a neutral with equal inks, where the device formulas put it on black
    // alone.
    let gray = press(synthetic_icc::srgb_gray_lightness(0.5), 0.0, 0.0);
    let got = plates(&pdf, &inks(&[]), true, Some(&simulation)).at(20, 20);
    assert!(near(&got, &gray, 2), "{got:?}, the press prints {gray:?}");
    assert_eq!(plates(&pdf, &inks(&[]), true, None).at(20, 20), [0, 0, 0, 50]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn only_cmyk_profiles_with_pcs_to_device_tables_simulate() {
    let whole = synthetic_icc::cmyk_profile(PRESS, true);
    assert_eq!(CmykSimulation::new(&synthetic_icc::gray_profile("Gray")).err(), Some(NoSimulation::NotCmyk));
    assert_eq!(CmykSimulation::new(&synthetic_icc::cmyk_profile("Proofing only", false)).err(), Some(NoSimulation::NoPcsToDevice));
    assert_eq!(CmykSimulation::new(b"not a profile").err(), Some(NoSimulation::Unreadable));
    assert_eq!(CmykSimulation::new(&whole[..300]).err(), Some(NoSimulation::Unreadable));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn cmyk_in_the_simulated_profile_passes_through_and_other_cmyk_converts() {
    let profile = synthetic_icc::cmyk_profile(PRESS, true);
    let simulation = CmykSimulation::new(&profile).unwrap();
    // Object 5 is the press profile, 6 another CMYK profile.
    let resources = "/ColorSpace << /Same [/ICCBased 5 0 R] /Other [/ICCBased 6 0 R] >>";
    let content = "0.2 0.4 0.6 0.8 k 0 0 10 40 re f /Same cs 0.1 0.3 0.5 0.7 scn 10 0 10 40 re f /Other cs 0 0 0 1 scn 30 0 10 40 re f";
    let pdf = page_pdf(content, resources, &[stream("/N 4", &profile), stream("/N 4", CGATS)]);
    let s = plates(&pdf, &inks(&[]), true, Some(&simulation));
    assert_eq!(s.at(5, 20), [20, 40, 60, 80], "DeviceCMYK is the press's own CMYK");
    assert_eq!(s.at(15, 20), [10, 30, 50, 70], "ICC CMYK in the simulated profile");
    let other = s.at(35, 20);
    assert!(other.iter().all(|v| *v > 70), "black alone in another profile is a four-ink black on this press: {other:?}");
    assert_eq!(plates(&pdf, &inks(&[]), true, None).at(35, 20), [0, 0, 0, 100], "without a simulation ICC CMYK is taken as it is");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn device_gray_stays_on_the_black_plate_under_a_simulation() {
    let s = plates(&page_pdf("0.25 g 0 0 40 40 re f", "", &[]), &inks(&[]), true, Some(&press_simulation()));
    assert_eq!(s.at(20, 20), [0, 0, 0, 75]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn cie_and_icc_colors_convert_through_the_profile_relative_to_their_white() {
    let resources =
        "/ColorSpace << /D50 [/Lab << /WhitePoint [0.9642 1 0.8249] >>] /D65 [/Lab << /WhitePoint [0.9505 1 1.089] >>] /Gray [/ICCBased 5 0 R] >>";
    let content = "/D50 cs 50 20 -10 scn 0 0 10 40 re f /D65 cs 100 0 0 scn 10 0 10 40 re f /Gray cs 0.5 scn 20 0 10 40 re f";
    let pdf = page_pdf(content, resources, &[stream("/N 1", &synthetic_icc::gray_profile("Gray 2.2"))]);
    let s = plates(&pdf, &inks(&[]), true, Some(&press_simulation()));
    assert!(near(&s.at(5, 20), &press(50.0, 20.0, -10.0), 2), "Lab: {:?}", s.at(5, 20));
    assert_eq!(s.at(15, 20), [0, 0, 0, 0], "a Lab space's own white prints as paper, whatever its white point");
    let lightness = 116.0 * 0.5_f64.powf(2.2).cbrt() - 16.0;
    assert!(near(&s.at(25, 20), &press(lightness, 0.0, 0.0), 2), "ICC gray: {:?}", s.at(25, 20));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn images_convert_through_the_simulation_sample_by_sample() {
    let rgb = stream("/Type /XObject /Subtype /Image /Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB", &[128, 128, 128, 255, 255, 255]);
    // More samples than palette entries: the palette is separated once.
    let indexed = stream(
        "/Type /XObject /Subtype /Image /Width 4 /Height 1 /BitsPerComponent 8 /ColorSpace [/Indexed /DeviceRGB 1 <808080FFFFFF>]",
        &[0, 0, 1, 1],
    );
    let content = "q 40 0 0 20 0 20 cm /Im1 Do Q q 40 0 0 20 0 0 cm /Im2 Do Q";
    let pdf = page_pdf(content, "/XObject << /Im1 5 0 R /Im2 6 0 R >>", &[rgb, indexed]);
    let s = plates(&pdf, &inks(&[]), true, Some(&press_simulation()));
    let gray = press(synthetic_icc::srgb_gray_lightness(128.0 / 255.0), 0.0, 0.0);
    for y in [10, 30] {
        assert!(near(&s.at(10, y), &gray, 2), "y = {y}: {:?}, the press prints {gray:?}", s.at(10, y));
        assert_eq!(s.at(30, y), [0, 0, 0, 0], "y = {y}: white prints no ink");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_simulation_shows_its_cmyk_through_its_own_profile() {
    // The inks the press prints sRGB gray 128 with show as that gray: its two tables invert
    // each other.
    let ink = (255.0 * (1.0 - synthetic_icc::srgb_gray_lightness(128.0 / 255.0) / 100.0)).round() as u8;
    let mut shown = [0; 3];
    press_simulation().cmyk_to_srgb8(&[ink; 4], &mut shown).unwrap();
    assert!(shown.iter().all(|c| c.abs_diff(128) <= 4), "{shown:?}");
    assert!(press_simulation().cmyk_to_srgb8(&[0; 4], &mut [0; 4]).is_none(), "lengths disagree");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn mutated_profiles_simulate_or_are_refused_without_panicking() {
    // The header fields a reader checks, every 16th byte of the tag table and bytes spread
    // over the tag data, changed in turn. Making a simulation takes a fifth of a second in a
    // debug build, so header bytes readers skip (date, creator, profile ID, reserved) are left
    // out and the data is sampled.
    let whole = synthetic_icc::cmyk_profile(PRESS, true);
    let pdf = page_pdf("0.5 0.5 0.5 rg 0 0 40 40 re f", "", &[]);
    // Size, version, class, color space, PCS, `acsp`, rendering intent, illuminant.
    let header = [3, 8, 12, 16, 20, 36, 67, 70];
    let positions = header.into_iter().chain((128..256).step_by(16)).chain((256..whole.len()).step_by(15_000));
    for (n, at_byte) in positions.enumerate() {
        let mut bent = whole.clone();
        bent[at_byte] = [0x00, 0x7F, 0xFF][n % 3];
        if let Ok(simulation) = CmykSimulation::new(&bent) {
            let plate = Plate { ink: Ink::Cyan, overprint: true, simulation: Some(simulation), spots_as_process: false };
            render(&pdf, &InterpreterSettings { plate: Some(plate), ..Default::default() });
        }
    }
}

/// A caller's conversion: sRGB prints as 10/20/30/40%, XYZ as 50/60/70/80%, and the XYZ it is
/// given is kept.
struct Fixed(Arc<Mutex<Vec<[f32; 3]>>>);

impl CmykConversion for Fixed {
    fn from_srgb(&self, rgb: &[f32], cmyk: &mut [f32]) -> Option<()> {
        (rgb.len() / 3 == cmyk.len() / 4).then(|| cmyk.as_chunks_mut::<4>().0.fill([0.1, 0.2, 0.3, 0.4]))
    }

    fn from_pcs_xyz(&self, xyz: &[f32], cmyk: &mut [f32]) -> Option<()> {
        self.0.lock().unwrap().extend(xyz.as_chunks::<3>().0.iter().copied());
        cmyk.as_chunks_mut::<4>().0.fill([0.5, 0.6, 0.7, 0.8]);
        Some(())
    }

    fn description(&self) -> Option<&str> {
        Some("fixed")
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_callers_cmyk_conversion_separates_process_color() {
    let profile = synthetic_icc::cmyk_profile(PRESS, true);
    let xyz = Arc::default();
    let simulation = CmykSimulation::with_conversion(&profile, Fixed(Arc::clone(&xyz)));
    assert_eq!(simulation.description(), Some("fixed"));
    assert!(simulation.cmyk_to_srgb8(&[0; 4], &mut [0; 3]).is_none(), "no way to show CMYK");
    let resources = "/ColorSpace << /Lab [/Lab << /WhitePoint [0.9505 1 1.089] >>] /Same [/ICCBased 5 0 R] >>";
    let content = "1 0 0 rg 0 0 10 40 re f /Lab cs 100 0 0 scn 10 0 10 40 re f 0.2 0.4 0.6 0.8 k 20 0 10 40 re f \
                   /Same cs 0.1 0.3 0.5 0.7 scn 30 0 10 40 re f";
    let s = plates(&page_pdf(content, resources, &[stream("/N 4", &profile)]), &inks(&[]), true, Some(&simulation));
    assert_eq!(s.at(5, 20), [10, 20, 30, 40], "DeviceRGB through from_srgb");
    assert_eq!(s.at(15, 20), [50, 60, 70, 80], "Lab through from_pcs_xyz");
    assert_eq!(s.at(25, 20), [20, 40, 60, 80], "DeviceCMYK as it is");
    assert_eq!(s.at(35, 20), [10, 30, 50, 70], "ICC CMYK in the simulated profile as it is");
    let xyz = xyz.lock().unwrap();
    assert!(!xyz.is_empty());
    for c in xyz.iter() {
        assert!(c.iter().zip(PCS_WHITE).all(|(v, w)| (v - w).abs() < 1e-3), "a Lab white arrives as the PCS white: {c:?}");
    }
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

/// PdfCraft #3018: `/DefaultCMYK` and `/DefaultGray` change how device color looks, not the
/// inks it prints with. Under them DeviceCMYK still separates as its components (and keeps the
/// nonzero overprint mode), a DeviceCMYK image sample by sample, and DeviceGray on black alone,
/// with or without a simulation whose profile differs from the default.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn default_color_spaces_leave_device_color_on_its_plates() {
    let image = stream("/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceCMYK", &[51, 102, 153, 204]);
    let objects = [stream("/N 4", CGATS), stream("/N 1", &synthetic_icc::gray_profile("Gray 2.2")), image];
    let content = "0 0 0 1 k 0 0 10 40 re f 1 0 0 0 k 10 0 10 40 re f /M1 gs 0 1 0 0 k 10 0 10 40 re f \
                   q 10 0 0 40 20 0 cm /Im1 Do Q 0.25 g 30 0 10 40 re f";
    let resources = "/ExtGState << /M1 << /OP true /OPM 1 >> >> /XObject << /Im1 7 0 R >>";
    let calibrated = format!("{resources} /ColorSpace << /DefaultCMYK [/ICCBased 5 0 R] /DefaultGray [/ICCBased 6 0 R] >>");
    let expected: [[u32; 4]; 4] = [[0, 0, 0, 100], [100, 100, 0, 0], [20, 40, 60, 80], [0, 0, 0, 75]];
    for simulation in [None, Some(press_simulation())] {
        let s = plates(&page_pdf(content, &calibrated, &objects), &inks(&[]), true, simulation.as_ref());
        for (x, want) in [5, 15, 25, 35].into_iter().zip(expected) {
            assert_eq!(s.at(x, 20), want, "x {x}, simulated: {}", simulation.is_some());
        }
    }
}
