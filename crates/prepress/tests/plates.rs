use std::sync::Arc;

use craft_cms::{Intent, Profile, Transform, builtin};
use craft_prepress::{Alternate, CmsConversion, Conversion, Error, Fallback, Ink, InkId, MAX_SPOT_INKS, Options, Process, plates};

/// A 40 × 40 pt page with `content` and `resources`, `catalog` entries, and `objects`
/// numbered from 5 (each a whole object body).
fn page_pdf(content: &str, resources: &str, catalog: &str, objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    out.extend_from_slice(format!("1 0 obj << /Type /Catalog /Pages 2 0 R {catalog} >> endobj\n").as_bytes());
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

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict} /Length {} >> stream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// A spot color whose alternate is a yellowish CMYK.
const GOLD: &str = "[/Separation /Gold /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0.3 1 0] /N 1 >>]";

/// Black over the page, Gold at 50% in the middle square, overprinting when `gs` sets `op`.
fn black_and_gold(gs: &str) -> Vec<u8> {
    let resources = format!("/ColorSpace << /CS0 {GOLD} >> /ExtGState << /OP << /op true >> >>");
    page_pdf(&format!("0 0 0 1 k 0 0 40 40 re f {gs} /CS0 cs 0.5 scn 10 10 20 20 re f"), &resources, "", &[])
}

fn black() -> InkId {
    InkId::process(Process::Black)
}

#[test]
fn a_black_and_gold_page_separates_into_the_process_plates_and_gold() {
    let p = plates(&black_and_gold(""), 0, 72.0, &Options::default()).unwrap();
    assert_eq!((p.width, p.height, p.dpi), (40, 40, 72.0));
    let names: Vec<&str> = p.inks.iter().map(|(_, ink)| ink.name()).collect();
    assert_eq!(names, ["Cyan", "Magenta", "Yellow", "Black", "Gold"]);
    assert_eq!(p.planes.len(), 5);
    assert!(p.unseparated.is_empty());
    let gold = p.inks.lookup("Gold").unwrap();
    let Some(Ink::Spot(spot)) = p.inks.get(gold) else { panic!("Gold is a spot ink") };
    let Alternate::Lab([l, _, b]) = spot.alternate else { panic!("a Lab alternate") };
    assert!(l > 60.0 && b > 40.0, "Gold looks yellow: {:?}", spot.alternate);

    let half = |t: Option<u8>| t.is_some_and(|t| t.abs_diff(128) <= 1);
    assert!(half(p.tint(gold, 20, 20)), "Gold at 50%");
    assert_eq!(p.tint(black(), 20, 20), Some(0), "Gold knocks out black");
    assert_eq!(p.tint(black(), 2, 2), Some(255));
    assert_eq!(p.tint(gold, 2, 2), Some(0));
    for c in [Process::Cyan, Process::Magenta, Process::Yellow] {
        assert!(p.plane(InkId::process(c)).unwrap().iter().all(|t| *t == 0), "{c:?} is empty");
    }

    let over = plates(&black_and_gold("/OP gs"), 0, 72.0, &Options::default()).unwrap();
    assert!(half(over.tint(gold, 20, 20)));
    assert_eq!(over.tint(black(), 20, 20), Some(255), "overprinted");
    let off = plates(&black_and_gold("/OP gs"), 0, 72.0, &Options { overprint: false, ..Options::default() }).unwrap();
    assert_eq!(off.tint(black(), 20, 20), Some(0), "overprint off: knocked out");

    // The preview reads the plates: knocked-out Gold shows pale on paper, overprinted Gold
    // darkens the black under it.
    let cms = CmsConversion::fogra39();
    let px = |rgb: &[u8], x: usize, y: usize| -> [u8; 3] { rgb[(y * 40 + x) * 3..][..3].try_into().unwrap() };
    let (knockout, overprint) = (p.composite(&[], &cms), over.composite(&[], &cms));
    let [r, g, b] = px(&knockout, 20, 20);
    assert!(r > 200 && g > 180 && b < g, "50% Gold on paper: {:?}", [r, g, b]);
    let k = px(&knockout, 2, 2);
    assert!(k.iter().all(|c| *c < 80), "black: {k:?}");
    assert_eq!(px(&overprint, 2, 2), k);
    let dark = px(&overprint, 20, 20);
    assert!(dark.iter().zip(k).all(|(d, k)| *d <= k) && dark[2] < k[2], "Gold over black: {dark:?}, black {k:?}");
}

/// What `profile` prints for sRGB red, as tint bytes.
fn red_in(profile: &[u8]) -> [u8; 4] {
    let t = Transform::new(builtin::srgb(), &Profile::parse(profile).unwrap(), Intent::RelativeColorimetric, false).unwrap();
    let mut cmyk = [0.0_f32; 4];
    t.convert_f32(&[1.0, 0.0, 0.0], 3, &mut cmyk, 4, false).unwrap();
    cmyk.map(|v| (v * 255.0).round() as u8)
}

fn red_tints(p: &craft_prepress::Plates) -> [u8; 4] {
    Process::ALL.map(|c| p.tint(InkId::process(c), 20, 20).unwrap())
}

fn near(got: [u8; 4], expected: [u8; 4]) -> bool {
    got.iter().zip(expected).all(|(g, e)| g.abs_diff(e) <= 3)
}

#[test]
fn process_color_converts_through_the_output_intent_else_the_fallback() {
    let synthetic = builtin::COATED_CMYK_ICC;
    let intent = "/OutputIntents [<< /Type /OutputIntent /S /GTS_PDFX /OutputConditionIdentifier (Synthetic press) /DestOutputProfile 5 0 R >>]";
    let red = |catalog: &str| page_pdf("1 0 0 rg 0 0 40 40 re f", "", catalog, &[stream("/N 4", synthetic)]);

    let p = plates(&red(intent), 0, 72.0, &Options::default()).unwrap();
    let profile = craft_cms::synth::DESCRIPTION.to_string();
    assert_eq!(p.conversion, Conversion::OutputIntent { profile: profile.clone(), condition: Some("Synthetic press".into()) });
    let expected = red_in(synthetic);
    assert!(near(red_tints(&p), expected), "{:?}, the output intent prints {expected:?}", red_tints(&p));

    let fogra = plates(&red(""), 0, 72.0, &Options::default()).unwrap();
    assert_eq!(fogra.conversion, Conversion::Fallback { profile: builtin::COATED_FOGRA39_DESCRIPTION.into() });
    let expected = red_in(builtin::COATED_FOGRA39_ICC);
    assert!(near(red_tints(&fogra), expected), "{:?}, FOGRA39 prints {expected:?}", red_tints(&fogra));

    let callers = Options { fallback: Fallback::Profile(Arc::from(synthetic)), ..Options::default() };
    assert_eq!(plates(&red(""), 0, 72.0, &callers).unwrap().conversion, Conversion::Fallback { profile });

    let formulas = Options { fallback: Fallback::DeviceFormulas, ..Options::default() };
    let device = plates(&red(""), 0, 72.0, &formulas).unwrap();
    assert_eq!((device.conversion.clone(), red_tints(&device)), (Conversion::DeviceFormulas, [0, 255, 255, 0]));

    let broken = Options { fallback: Fallback::Profile(Arc::from(&b"not a profile"[..])), ..Options::default() };
    assert!(matches!(plates(&red(""), 0, 72.0, &broken), Err(Error::Profile(_))));
}

#[test]
fn spot_inks_past_the_cap_are_listed_not_separated_and_bad_input_is_refused() {
    let n = MAX_SPOT_INKS + 2;
    let spaces: String =
        (0..n).map(|i| format!("/S{i} [/Separation /Spot{i} /DeviceGray << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >>] ")).collect();
    let content: String = (0..n).map(|i| format!("/S{i} cs 1 scn {i} 0 1 1 re f ")).collect();
    let pdf = page_pdf(&content, &format!("/ColorSpace << {spaces} >>"), "", &[]);
    let p = plates(&pdf, 0, 72.0, &Options::default()).unwrap();
    assert_eq!((p.inks.len(), p.planes.len()), (4 + MAX_SPOT_INKS, 4 + MAX_SPOT_INKS));
    assert_eq!(p.unseparated, [format!("Spot{}", n - 2), format!("Spot{}", n - 1)]);

    assert!(matches!(plates(&pdf, 1, 72.0, &Options::default()), Err(Error::NoPage(1))));
    assert!(matches!(plates(&pdf, 0, f32::NAN, &Options::default()), Err(Error::InvalidDpi)));
    assert!(matches!(plates(b"not a pdf", 0, 72.0, &Options::default()), Err(Error::Open(_))));
}

#[test]
fn escaped_spot_names_are_decoded_once_and_each_gets_its_own_plate() {
    // `/A#2341` is the spot `A#41`, not `AA`; `/Spot#20#2312` is `Spot #12`.
    let names = ["/A#2341", "/AA", "/Spot#20#2312"];
    let spaces: String = names
        .iter()
        .enumerate()
        .map(|(i, name)| format!("/S{i} [/Separation {name} /DeviceGray << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >>] "))
        .collect();
    let content: String = (0..names.len()).map(|i| format!("/S{i} cs 1 scn {} 0 10 10 re f ", i * 10)).collect();
    let pdf = page_pdf(&content, &format!("/ColorSpace << {spaces} >>"), "", &[]);
    let p = plates(&pdf, 0, 72.0, &Options::default()).unwrap();
    let spots: Vec<&str> = p.inks.spots().map(|(_, s)| s.name.as_str()).collect();
    assert_eq!(spots, ["A#41", "AA", "Spot #12"]);
    assert!(p.unseparated.is_empty());
    assert_eq!(p.planes.len(), 7);
    for (i, (id, _)) in p.inks.spots().enumerate() {
        for x in [5u32, 15, 25] {
            let expected = if (x / 10) as usize == i { 255 } else { 0 };
            assert_eq!(p.tint(id, x, 35), Some(expected), "spot {i} at x = {x}");
        }
    }
}

#[test]
fn lab_colors_convert_through_the_profile_and_lab_white_is_paper() {
    let resources = "/ColorSpace << /D50 [/Lab << /WhitePoint [0.9642 1 0.8249] >>] /D65 [/Lab << /WhitePoint [0.9505 1 1.089] >>] >>";
    let pdf = page_pdf("/D50 cs 50 20 -10 scn 0 0 20 40 re f /D65 cs 100 0 0 scn 20 0 20 40 re f", resources, "", &[]);
    let p = plates(&pdf, 0, 72.0, &Options::default()).unwrap();
    let at = |x| Process::ALL.map(|c| p.tint(InkId::process(c), x, 20).unwrap());
    let fogra = Profile::parse(builtin::COATED_FOGRA39_ICC).unwrap();
    let t = Transform::new(craft_cms::Builtin::LabD50.profile(), &fogra, Intent::RelativeColorimetric, false).unwrap();
    let mut cmyk = [0.0_f32; 4];
    t.convert_f32(&[0.5, 148.0 / 255.0, 118.0 / 255.0], 3, &mut cmyk, 4, false).unwrap();
    let expected = cmyk.map(|v| (v * 255.0).round() as u8);
    assert!(near(at(10), expected), "{:?}, FOGRA39 prints {expected:?}", at(10));
    assert_eq!(at(30), [0, 0, 0, 0], "a Lab space's own white prints as paper");
}

/// A 60 × 10 pt page written by the PDF writer, with a 10 pt wide fill of each color.
fn written(colors: Vec<craft_pdf::write::backend::color::Color>) -> Vec<u8> {
    use craft_pdf::write::backend::{geom, page::PageSettings, paint::Fill};
    let mut writer = craft_pdf::write::Writer::new(Default::default(), Default::default());
    let mut page = writer.document_mut().start_page_with(PageSettings::new(geom::Size::from_wh(60.0, 10.0).unwrap()));
    let mut surface = page.surface();
    for (i, color) in colors.into_iter().enumerate() {
        let mut path = geom::PathBuilder::new();
        path.push_rect(geom::Rect::from_xywh(10.0 * i as f32, 0.0, 10.0, 10.0).unwrap());
        surface.set_fill(Some(Fill { paint: color.into(), ..Default::default() }));
        surface.draw_path(&path.finish().unwrap());
    }
    surface.finish();
    page.finish();
    writer.finish().unwrap()
}

/// The page's color space resources as (family, colorant names), sorted.
fn color_spaces(pdf: &[u8]) -> Vec<(String, Vec<String>)> {
    use craft_pdf::syntax::object::{Array, Name, Object};
    let pdf = craft_pdf::syntax::Pdf::new(Arc::new(pdf.to_vec())).unwrap();
    let page = pdf.pages().iter().next().unwrap();
    let spaces = &page.resources().color_spaces;
    let mut out: Vec<(String, Vec<String>)> = spaces
        .keys()
        .map(|key| {
            let array = spaces.get::<Array<'_>>(key.as_ref()).unwrap();
            let items: Vec<Object<'_>> = array.iter::<Object<'_>>().collect();
            let Some(Object::Name(family)) = items.first() else { panic!("a color space without a family") };
            let names = match items.get(1) {
                Some(Object::Name(n)) => vec![n.as_str().to_owned()],
                Some(Object::Array(a)) => a.iter::<Name<'_>>().map(|n| n.as_str().to_owned()).collect(),
                _ => Vec::new(),
            };
            (family.as_str().to_owned(), names)
        })
        .collect();
    out.sort();
    out
}

#[test]
fn ink_colors_write_as_cmyk_separation_or_device_n_and_separate_onto_their_plates() {
    use craft_prepress::{InkColor, InkSet, Spot, to_pdf_color};
    let mut inks = InkSet::new();
    let gold = inks.add(Ink::Spot(Spot { name: "Gold".into(), alternate: Alternate::Cmyk([0.0, 0.3, 1.0, 0.0]), solidity: None })).unwrap();
    let teal = inks.add(Ink::Spot(Spot { name: "Teal".into(), alternate: Alternate::Lab([55.0, -40.0, -10.0]), solidity: None })).unwrap();
    let (c, k) = (InkId::process(Process::Cyan), black());
    let cms = CmsConversion::fogra39();
    let colors = [
        InkColor::Tints(vec![(k, 0.4), (c, 0.6)]),
        InkColor::Tints(vec![(gold, 0.5)]),
        InkColor::Tints(vec![(gold, 1.0), (k, 0.5)]),
        InkColor::Tints(vec![(teal, 1.0)]),
        InkColor::All(1.0),
        InkColor::None,
    ];
    let pdf = written(colors.iter().map(|color| to_pdf_color(color, &inks, &cms).unwrap()).collect());
    let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let mut expected = vec![
        ("DeviceN".to_owned(), names(&["Black", "Gold"])),
        ("Separation".to_owned(), names(&["All"])),
        ("Separation".to_owned(), names(&["Gold"])),
        ("Separation".to_owned(), names(&["None"])),
        ("Separation".to_owned(), names(&["Teal"])),
    ];
    expected.sort();
    assert_eq!(color_spaces(&pdf), expected, "process only is DeviceCMYK, which needs no resource");

    let p = plates(&pdf, 0, 72.0, &Options::default()).unwrap();
    let names: Vec<&str> = p.inks.iter().map(|(_, ink)| ink.name()).collect();
    assert_eq!(names, ["Cyan", "Magenta", "Yellow", "Black", "Gold", "Teal"]);
    let (gold, teal) = (p.inks.lookup("Gold").unwrap(), p.inks.lookup("Teal").unwrap());
    let at = |x: u32| -> Vec<u8> { p.inks.iter().map(|(id, _)| p.tint(id, x, 5).unwrap()).collect() };
    assert_eq!(at(5), [153, 0, 0, 102, 0, 0]);
    assert_eq!(at(15), [0, 0, 0, 0, 128, 0]);
    assert_eq!(at(25), [0, 0, 0, 128, 255, 0]);
    assert_eq!(at(35), [0, 0, 0, 0, 0, 255]);
    assert_eq!(at(45), [255; 6], "registration");
    assert_eq!(at(55), [0; 6]);
    assert_eq!((p.tint(gold, 25, 5), p.tint(teal, 35, 5)), (Some(255), Some(255)));

    let unknown = InkSet::new();
    assert_eq!(to_pdf_color(&colors[1], &unknown, &cms), None, "an ink not in the set");
    assert_eq!(to_pdf_color(&InkColor::Tints(vec![]), &inks, &cms), None, "no ink: nothing to paint");
    let mut many = InkSet::new();
    let tints: Vec<(InkId, f32)> = (0..33)
        .map(|i| (many.add(Ink::Spot(Spot { name: format!("S{i}"), alternate: Alternate::Cmyk([0.0; 4]), solidity: None })).unwrap(), 1.0))
        .collect();
    assert_eq!(to_pdf_color(&InkColor::Tints(tints), &many, &cms), None, "more than 32 colorants");
}
