//! Tests for the swatch file codecs.

use super::reader::ASE_COLOR;
use super::*;
use craft_ui::testing::trial;

fn values(c: &SwatchColor) -> Vec<f32> {
    match *c {
        SwatchColor::Rgb(v) | SwatchColor::Hsb(v) | SwatchColor::Lab(v) => v.to_vec(),
        SwatchColor::Cmyk(v) => v.to_vec(),
        SwatchColor::Gray(k) => vec![k],
        SwatchColor::Book { .. } => Vec::new(),
    }
}

fn assert_same(a: &SwatchColor, b: &SwatchColor) {
    assert_eq!(std::mem::discriminant(a), std::mem::discriminant(b), "{a:?} vs {b:?}");
    if let (SwatchColor::Book { .. }, SwatchColor::Book { .. }) = (a, b) {
        assert_eq!(a, b);
    }
    for (x, y) in values(a).into_iter().zip(values(b)) {
        assert!((x - y).abs() < 1e-3, "{a:?} vs {b:?}");
    }
}

fn group(name: &str, entries: Vec<SwatchEntry>) -> SwatchGroup {
    SwatchGroup { name: name.into(), entries }
}

/// An `.aco` file with RGB, CMYK and Lab swatches round-trips with each colour in its model.
#[trial]
fn aco_round_trips_colours_in_their_own_model() {
    let colors = [
        ("Red", SwatchColor::Rgb([1.0, 0.0, 0.25])),
        ("Hue", SwatchColor::Hsb([120.0, 0.5, 0.75])),
        ("Cyan ink", SwatchColor::Cmyk([1.0, 0.0, 0.25, 0.1])),
        ("Lab", SwatchColor::Lab([53.0, -80.0, 67.5])),
        ("Gray", SwatchColor::Gray(0.4)),
        ("Book 日本", SwatchColor::Book { space: 3, values: [1, 2, 3, 4] }),
    ];
    let entries = colors.iter().map(|(n, c)| SwatchEntry::new(*n, *c)).collect();
    let (bytes, report) = write_swatches(&[group("All", entries)], SwatchFormat::Aco).unwrap();
    assert!(report.notes.is_empty(), "{report:?}");
    let (groups, report) = read_swatches(&bytes, "Brand.aco").unwrap();
    assert_eq!(report.notes, [SwatchNote::BookColors(1)]);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].name, "Brand");
    for ((name, color), entry) in colors.iter().zip(&groups[0].entries) {
        assert_eq!(entry.name, *name);
        assert_same(color, entry.color().unwrap());
    }
    // A version 1 section comes first, then version 2 with the names.
    assert_eq!(&bytes[..4], &[0, 1, 0, 6]);
    assert!(bytes.windows(4).any(|w| w == [0, 2, 0, 6]));
}

/// `.ase` keeps groups and spot colours; `.aco` flattens them and says so.
#[trial]
fn ase_keeps_groups_and_aco_reports_flattening() {
    let mut spot = SwatchEntry::new("Spot Orange", SwatchColor::Cmyk([0.0, 0.5, 1.0, 0.0]));
    spot.kind = SwatchKind::Spot;
    let groups = vec![
        group("Brand", vec![SwatchEntry::new("Blue", SwatchColor::Rgb([0.0, 0.0, 1.0])), spot]),
        group("Greys", vec![SwatchEntry::new("Mid", SwatchColor::Gray(0.5))]),
    ];
    let (ase, report) = write_swatches(&groups, SwatchFormat::Ase).unwrap();
    assert!(report.notes.is_empty());
    let (back, _) = read_swatches(&ase, "x.ase").unwrap();
    assert_eq!(back.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Brand", "Greys"]);
    assert_eq!(back[0].entries[1].kind, SwatchKind::Spot);
    assert_same(back[0].entries[1].color().unwrap(), groups[0].entries[1].color().unwrap());
    assert_same(back[1].entries[0].color().unwrap(), &SwatchColor::Gray(0.5));

    // A colour in a model this crate does not read is skipped and counted.
    let mut unknown = Vec::new();
    put_ase_name(&mut unknown, "Odd");
    unknown.extend(b"XYZ ");
    unknown.extend([0u8; 12]);
    unknown.extend(2u16.to_be_bytes());
    let mut file = ase.clone();
    put_ase_block(&mut file, ASE_COLOR, &unknown);
    file[8..12].copy_from_slice(&(u32::from_be_bytes(ase[8..12].try_into().unwrap()) + 1).to_be_bytes());
    let (skipped, report) = read_swatches(&file, "x.ase").unwrap();
    assert_eq!(report.notes, [SwatchNote::Skipped(1)]);
    assert_eq!(count_swatches(&skipped), 3);

    let (aco, report) = write_swatches(&groups, SwatchFormat::Aco).unwrap();
    assert_eq!(report.notes, [SwatchNote::GroupsFlattened, SwatchNote::KindsDropped(1)]);
    let (flat, _) = read_swatches(&aco, "x.aco").unwrap();
    assert_eq!(flat.len(), 1);
    assert_eq!(flat[0].entries.len(), 3);
}

/// Every writer leaves out the swatches that have no colour and counts them once, after its other notes; colours,
/// tints and the Registration swatch are written. A library of paints alone is empty. An entry stored without a
/// paint reads as a colour.
#[trial]
fn writers_skip_paint_swatches_and_report_them() {
    let mut registration = SwatchEntry::new("Registration", SwatchColor::Cmyk([1.0; 4]));
    registration.kind = SwatchKind::Registration;
    let paints = vec![
        SwatchEntry::with_paint("Sky", SwatchPaint::Gradient),
        SwatchEntry::with_paint("Dots", SwatchPaint::Pattern),
        SwatchEntry::with_paint("None", SwatchPaint::None),
    ];
    let mut entries = vec![
        SwatchEntry::new("Red", SwatchColor::Rgb([1.0, 0.0, 0.0])),
        SwatchEntry::with_paint("Pink", SwatchPaint::Tint { base: "Red".into(), tint: 0.5, color: SwatchColor::Rgb([1.0, 0.5, 0.5]) }),
        registration,
    ];
    entries.extend(paints.clone());
    let groups = vec![group("All", entries)];
    for format in SwatchFormat::ALL {
        let (bytes, report) = write_swatches(&groups, format).unwrap();
        let counted = report.notes.iter().filter(|n| matches!(n, SwatchNote::PaintsSkipped(_))).count();
        assert_eq!(counted, 1, "{format:?}: {report:?}");
        assert_eq!(report.notes.last(), Some(&SwatchNote::PaintsSkipped(3)), "{format:?}: {report:?}");
        if format.can_read() {
            let (back, _) = read_swatches(&bytes, &format!("x.{}", format.extension())).unwrap();
            assert_eq!(count_swatches(&back), 3, "{format:?}");
        }
        assert_eq!(write_swatches(&[group("Paints", paints.clone())], format).unwrap_err(), SwatchError::Empty, "{format:?}");
    }

    let legacy: SwatchEntry = serde_json::from_str(r#"{"name":"Old","color":{"model":"rgb","values":[1.0,0.0,0.0]},"kind":"spot"}"#).unwrap();
    assert_eq!(legacy.paint, SwatchPaint::Color(SwatchColor::Rgb([1.0, 0.0, 0.0])));
    assert_eq!(legacy.kind, SwatchKind::Spot);
    for entry in &groups[0].entries {
        let json = serde_json::to_string(entry).unwrap();
        assert_eq!(serde_json::from_str::<SwatchEntry>(&json).unwrap(), *entry, "{json}");
    }
}
