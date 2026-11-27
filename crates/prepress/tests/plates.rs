//! Integration tests of craft-prepress: plates.

use std::sync::Arc;

use craft_cms::{Profile, builtin};
use craft_prepress::{Alternate, CmsConversion, Error, Fallback, Ink, InkId, Options, Process, plates, press_profile};

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

/// A spot color whose alternate is a yellowish CMYK.
const GOLD: &str = "[/Separation /Gold /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0.3 1 0] /N 1 >>]";

/// Black over the page, Gold at 50% in the middle square, overprinting when `gs` sets `op`.
fn black_and_gold(gs: &str) -> Vec<u8> {
    let resources = format!("/ColorSpace << /CS0 {GOLD} >> /ExtGState << /OP << /op true >> >>");
    page_pdf(&format!("0 0 0 1 k 0 0 40 40 re f {gs} /CS0 cs 0.5 scn 10 10 20 20 re f"), &resources, "", &[])
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict} /Length {} >> stream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
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

#[test]
fn the_press_profile_is_the_one_the_plates_separate_through() {
    let synthetic = builtin::COATED_CMYK_ICC;
    let intent = "/OutputIntents [<< /Type /OutputIntent /S /GTS_PDFX /DestOutputProfile 5 0 R >>]";
    let red = |catalog: &str| page_pdf("1 0 0 rg 0 0 40 40 re f", "", catalog, &[stream("/N 4", synthetic)]);
    let parsed = |icc: &[u8]| Some(Profile::parse(icc).unwrap());
    let defaults = Options::default();

    assert_eq!(press_profile(&red(intent), 0, &defaults).unwrap(), parsed(synthetic), "the output intent's");
    assert_eq!(press_profile(&red(""), 0, &defaults).unwrap(), parsed(builtin::COATED_FOGRA39_ICC), "FOGRA39");
    let callers = Options { fallback: Fallback::Profile(Arc::from(synthetic)), ..Options::default() };
    assert_eq!(press_profile(&red(""), 0, &callers).unwrap(), parsed(synthetic), "the caller's");
    let formulas = Options { fallback: Fallback::DeviceFormulas, ..Options::default() };
    assert_eq!(press_profile(&red(""), 0, &formulas).unwrap(), None, "device formulas have no profile");
    assert_eq!(press_profile(&red(intent), 0, &formulas).unwrap(), parsed(synthetic), "an output intent still wins");
    assert!(matches!(press_profile(&red(""), 1, &defaults), Err(Error::NoPage(1))));
    let broken = Options { fallback: Fallback::Profile(Arc::from(&b"not a profile"[..])), ..Options::default() };
    assert!(matches!(press_profile(&red(""), 0, &broken), Err(Error::Profile(_))));
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
