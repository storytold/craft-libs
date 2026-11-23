use craft_cms::{Builtin, Intent, Profile, builtin};
use craft_prepress::{MAX_PLATE_PIXELS, Options, Output, plates};
use craft_press_print::{Error, PressPage, Settings, print, render};
use craft_print::rgb;

/// A one-page PDF of `w` × `h` pt with `content`, `resources` and `catalog` entries, and
/// `objects` numbered from 5 (each a whole object body).
fn page_pdf(w: u32, h: u32, content: &str, resources: &str, catalog: &str, objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    out.extend_from_slice(format!("1 0 obj << /Type /Catalog /Pages 2 0 R {catalog} >> endobj\n").as_bytes());
    out.extend_from_slice(b"2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n");
    out.extend_from_slice(
        format!("3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Contents 4 0 R /Resources << {resources} >> >> endobj\n").as_bytes(),
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

fn srgb() -> Settings {
    Settings::new(builtin::srgb().clone())
}

fn px(page: &PressPage, x: u32, y: u32) -> [u8; 3] {
    let at = ((y * page.width + x) * 3) as usize;
    page.rgb[at..at + 3].try_into().unwrap()
}

/// Yellow paper, a 50% cyan bar that overprints it (x 10..20) and one that knocks it out
/// (x 25..35), both the full height of the page, so every row is the same. Overprint mode 1
/// leaves the yellow under the zero components of the overprinting cyan.
const BARS: &str = "0 0 1 0 k 0 0 40 40 re f /OP gs 0.5 0 0 0 k 10 0 10 40 re f /KO gs 25 0 10 40 re f";
const BARS_RESOURCES: &str = "/ExtGState << /OP << /op true /OP true /OPM 1 >> /KO << /op false /OP false >> >>";

/// Yellow paper with cyan over its top half: in device rows, cyan down to the middle, then
/// yellow.
const HALVES: &str = "0 0 1 0 k 0 0 40 40 re f 1 0 0 0 k 0 20 40 20 re f";

#[test]
fn bands_join_without_a_seam_into_one_page_image() {
    let pdf = page_pdf(40, 40, HALVES, "", "", &[]);
    let page = render(&pdf, 0, 3700.0, &srgb()).unwrap();
    assert_eq!(page.rgb.len(), (page.width * page.height * 3) as usize);
    assert!((page.width * page.height) as f32 > MAX_PLATE_PIXELS, "more than one band: {} × {}", page.width, page.height);
    let rows: Vec<&[u8]> = page.rgb.chunks_exact((page.width * 3) as usize).collect();
    let (top, bottom) = (rows[0], rows[rows.len() - 1]);
    assert_ne!(top, bottom);
    // Every row is the top's or the bottom's, except at most one at the edge, which is in the
    // middle of the page.
    let cyan = rows.iter().take_while(|r| **r == top).count();
    let yellow = rows.iter().rev().take_while(|r| **r == bottom).count();
    assert!(cyan + yellow + 1 >= rows.len(), "{cyan} cyan and {yellow} yellow rows of {}", rows.len());
    assert!(cyan.abs_diff(rows.len() / 2) <= 1, "the edge at row {cyan} of {}", rows.len());
}

#[test]
fn overprint_reaches_the_printer_pixels() {
    let pdf = page_pdf(40, 40, BARS, BARS_RESOURCES, "", &[]);
    let page = render(&pdf, 0, 72.0, &srgb()).unwrap();
    let wide = |p: [u8; 3]| p.map(i32::from);
    let [r, g, b] = wide(px(&page, 15, 20));
    assert!(g > r && g > b + 60, "cyan over yellow prints green: {:?}", [r, g, b]);
    let [r, g, b] = wide(px(&page, 30, 20));
    assert!(b > g - 40 && b > r + 60, "knocked-out cyan prints cyan: {:?}", [r, g, b]);

    let off = Settings { separation: Options { overprint: false, ..Options::default() }, ..srgb() };
    let knocked = render(&pdf, 0, 72.0, &off).unwrap();
    assert_eq!(px(&knocked, 15, 20), px(&knocked, 30, 20), "with overprint off both bars knock out");
}

#[test]
fn the_plates_convert_from_the_output_intents_profile() {
    let intent = "/OutputIntents [<< /Type /OutputIntent /S /GTS_PDFX /DestOutputProfile 5 0 R >>]";
    let icc = builtin::COATED_CMYK_ICC;
    let mut profile = format!("<< /N 4 /Length {} >> stream\n", icc.len()).into_bytes();
    profile.extend_from_slice(icc);
    profile.extend_from_slice(b"\nendstream");
    let pdf = page_pdf(40, 40, "1 0 0 rg 0 0 20 40 re f 0.2 0.6 0.9 0.1 k 20 0 20 40 re f", "", intent, &[profile]);

    let page = render(&pdf, 0, 72.0, &srgb()).unwrap();
    let plates = plates(&pdf, 0, 72.0, &Options::default()).unwrap();
    let synthetic = Profile::parse(icc).unwrap();
    let through = |press: &Profile| plates.composite_to(&[], &Output::new(press, builtin::srgb(), Intent::RelativeColorimetric, true).unwrap());
    assert_ne!(through(&synthetic), through(Builtin::CoatedFogra39.profile()), "the two presses differ here");
    assert_eq!(page.rgb, through(&synthetic), "the press is the output intent's");
}

#[test]
fn bad_input_is_refused_before_anything_prints() {
    let pdf = page_pdf(40, 40, BARS, BARS_RESOURCES, "", &[]);
    let cmyk = Settings::new(Builtin::CoatedFogra39.profile().clone());
    assert!(matches!(render(&pdf, 0, 72.0, &cmyk), Err(Error::PrinterNotRgb)));
    assert!(matches!(render(&pdf, 0, f32::NAN, &srgb()), Err(Error::Separate(_))));
    assert!(matches!(render(&pdf, 3, 72.0, &srgb()), Err(Error::Separate(_))));
    assert!(matches!(render(b"not a pdf", 0, 72.0, &srgb()), Err(Error::Separate(_))));

    // No such printer: refused while asking for its page, before a job starts.
    let job = rgb::Job {
        printer: "Craft press print: no such printer",
        settings: None,
        title: "press",
        output_file: None,
        copies: 1,
        color_policy: rgb::ColorPolicy::DriverManaged,
        primitives: &[],
    };
    assert!(matches!(print(&job, &pdf, 0, &srgb(), (0.0, 0.0, 40.0, 40.0)), Err(Error::Print(_))));
}
