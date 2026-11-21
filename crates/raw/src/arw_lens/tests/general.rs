//! The general conversion: one rule for every body and mode without a validated conversion.
use super::*;

fn frame(storage: (usize, usize), reference: Rect, compression: u16) -> SonyLensGeometry {
    SonyLensGeometry {
        storage_width: storage.0,
        storage_height: storage.1,
        sony_crop: Some(reference),
        default_crop: Some(reference),
        image_crop: reference,
        reference,
        compression,
        linear_ycbcr: false,
        native_3_2: true,
    }
}

/// The frames of the independent measurements: A7R III and A7R V, full frame (16 knots) and APS-C (11 knots).
fn measured(mode: &str) -> (SonyLensGeometry, u8, [i16; 16]) {
    let (storage, reference, compression, count, natural) = match mode {
        "r3-full-c" => {
            ((8000, 5320), Rect::new(8, 8, 7952, 5304), 32767, 16, [0, 0, -2, -4, -7, -11, -16, -22, -31, -41, -53, -66, -82, -99, -118, -139])
        }
        "r5-full-c" => {
            ((9600, 6376), Rect::new(32, 20, 9504, 6336), 32767, 16, [3, 0, -5, -12, -22, -32, -46, -57, -69, -76, -79, -70, -48, -4, 62, 168])
        }
        "r3-apsc-c" => ((5216, 3464), Rect::new(8, 8, 5168, 3448), 32767, 11, [-1, 1, 6, 14, 25, 39, 56, 76, 100, 127, 157, 0, 0, 0, 0, 0]),
        "r5-apsc-c" => ((6304, 4180), Rect::new(16, 10, 6240, 4160), 32767, 11, [3, 0, -5, -12, -22, -32, -46, -58, -70, -77, -79, 0, 0, 0, 0, 0]),
        "r5-apsc-u" => ((6304, 4180), Rect::new(16, 10, 6240, 4160), 1, 11, [3, 0, -5, -12, -22, -32, -46, -58, -70, -77, -79, 0, 0, 0, 0, 0]),
        _ => panic!("unknown mode {mode}"),
    };
    (frame(storage, reference, compression), count, natural)
}

/// The table of a control: the lens's own curve, or a synthetic quadratic of either sign. Storage padding past the
/// active knots holds a value that must never reach the map.
fn control(natural: [i16; 16], variant: &str, count: u8) -> [i16; 16] {
    std::array::from_fn(|i| {
        if i >= usize::from(count) {
            31000
        } else if variant == "natural" {
            natural[i]
        } else {
            (48.0 * (i as f64 / f64::from(count - 1)).powi(2)).round() as i16 * if variant == "plus" { 1 } else { -1 }
        }
    })
}

/// Source position, in stored-image pixels, of the output pixel at (`x`, `y`).
fn source(op: &Opcode, g: &SonyLensGeometry, x: f64, y: f64) -> (f64, f64) {
    let (ax, ay) = (g.storage_width as f64 - 1.0, g.storage_height as f64 - 1.0);
    let (center, factor): ([f64; 2], Box<dyn Fn(f64) -> f64>) = match op {
        Opcode::WarpRectilinear { planes, center } => {
            assert_eq!(planes.len(), 1, "distortion must not duplicate sensor CA");
            let k = planes[0];
            (*center, Box::new(move |r: f64| k[0] + r * r * (k[1] + r * r * (k[2] + r * r * k[3]))))
        }
        Opcode::WarpRectilinear2 { planes, center, reciprocal, .. } => {
            assert_eq!(planes.len(), 1, "distortion must not duplicate sensor CA");
            let (k, reciprocal) = (planes[0], *reciprocal);
            (*center, Box::new(move |r: f64| crate::opcodes::wr2_factor(&k, reciprocal, r).unwrap()))
        }
        other => panic!("not a radial warp: {other:?}"),
    };
    let (cx, cy) = (center[0] * ax, center[1] * ay);
    let farthest = cx.max(ax - cx).hypot(cy.max(ay - cy));
    let (dx, dy) = (x - cx, y - cy);
    let f = factor(dx.hypot(dy) / farthest);
    (cx + dx * f, cy + dy * f)
}

/// Sony's own converter, given containers that differ only in the distortion table, moved 247 positions per control.
/// The general conversion lands within a third of a pixel of every one at a 1440-pixel-wide preview.
#[test]
fn general_conversion_matches_independent_sony_exports() {
    let data = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sony/sony-distortion-native-2026-11.tsv"));
    for mode in ["r3-full-c", "r5-full-c", "r3-apsc-c", "r5-apsc-c", "r5-apsc-u"] {
        let (g, count, natural) = measured(mode);
        for variant in ["plus", "minus", "natural"] {
            let op = general_conversion(&control(natural, variant, count), count, &g).unwrap_or_else(|| panic!("{mode} {variant}"));
            let (rx, ry) = (g.reference.x as f64, g.reference.y as f64);
            let mut errors = Vec::new();
            for line in data.lines().skip(1) {
                let v: Vec<_> = line.split('\t').collect();
                if v[0] != mode || v[1] != variant {
                    continue;
                }
                let f = |i: usize| v[i].parse::<f64>().unwrap();
                assert_eq!((f(2) as usize, f(3) as usize), (g.reference.width, g.reference.height));
                let (x, y) = (f(5) + rx, f(6) + ry);
                let (sx, sy) = source(&op, &g, x, y);
                errors.push((sx - x - f(7)).hypot(sy - y - f(8)) * 1440.0 / g.reference.width as f64);
            }
            errors.sort_by(f64::total_cmp);
            assert_eq!(errors.len(), 247);
            assert!(errors[123] < 0.08 && errors[246] < 0.30, "{mode} {variant}: median {} max {} at 1440 px", errors[123], errors[246]);
        }
    }
}

/// A table is normalised to the sensor's 3:2 frame: an aspect crop of it keeps that frame's warp and centre.
#[test]
fn an_aspect_crop_keeps_the_sensor_frames_warp() {
    let reference = Rect::new(12, 12, 6000, 4000);
    let full = frame((6048, 4024), reference, 32767);
    let table = [0, -4, -15, -34, -60, -94, -135, -184, -240, -304, -375, 0, 0, 0, 0, 0];
    let warp = general_conversion(&table, 11, &full).unwrap();
    // 16:9 selected by Sony's crop tags, the default crop still naming the 3:2 frame
    let mut tagged = full.clone();
    tagged.image_crop = Rect::new(12, 324, 6000, 3376);
    tagged.sony_crop = Some(tagged.image_crop);
    tagged.native_3_2 = false;
    assert_eq!(general_conversion(&table, 11, &tagged), Some(warp));
    // an early body's untagged 16:9 raw: the 3:2 frame of its width about the crop
    let mut untagged = full.clone();
    untagged.image_crop = Rect::new(0, 324, 6000, 3376);
    untagged.reference = untagged.image_crop;
    (untagged.sony_crop, untagged.default_crop, untagged.native_3_2) = (None, None, false);
    assert_eq!(optical_reference(&untagged), Some(Rect::new(0, 12, 6000, 4000)));
    let Some(Opcode::WarpRectilinear { center, .. }) = general_conversion(&table, 11, &untagged) else { panic!("warp") };
    assert!((center[1] * 4023.0 - (12.0 + 3999.0 / 2.0)).abs() < 1e-9);
}

#[test]
fn frames_and_curves_outside_the_rule_get_no_warp() {
    let reference = Rect::new(12, 12, 6000, 4000);
    let full = frame((6048, 4024), reference, 32767);
    let table: [i16; 16] = std::array::from_fn(|i| (i * i) as i16 * 3);
    assert!(general_conversion(&table, 16, &full).is_some());
    // an in-camera crop mode inside the full stored frame may keep the full sensor's radius
    let mut crop_mode = full.clone();
    (crop_mode.image_crop, crop_mode.reference) = (Rect::new(1032, 688, 3984, 2656), Rect::new(1032, 688, 3984, 2656));
    assert!(general_conversion(&table, 16, &crop_mode).is_none());
    // a 4:3 sensor: its reference fills the stored image
    let compact = frame((4928, 3708), Rect::new(12, 18, 4896, 3672), 32767);
    assert!(general_conversion(&table, 11, &compact).is_some());
    // an aspect crop that Sony's tags describe without a 3:2 reference
    let mut odd = full.clone();
    odd.reference = Rect::new(12, 324, 6000, 3376);
    odd.native_3_2 = false;
    assert!(general_conversion(&table, 16, &odd).is_none());
    // a reference outside the stored image
    let mut outside = full.clone();
    outside.reference.x = 60;
    assert!(general_conversion(&table, 16, &outside).is_none());
    assert!(general_conversion(&table, 12, &full).is_none(), "an unknown knot count");
    assert!(general_conversion(&[0; 16], 16, &full).is_none(), "a neutral table");
    let mut strong = table;
    strong[15] = 4097;
    assert!(general_conversion(&strong, 16, &full).is_none(), "beyond ±25 %");
    // a fold: the picture would turn back on itself
    let mut fold = [0; 16];
    fold[8] = -4096;
    assert!(general_conversion(&fold, 16, &full).is_none());
}

/// A curve the four-term warp cannot follow takes the higher-order one, under the same accuracy bound.
#[test]
fn a_moustache_curve_takes_the_higher_order_warp() {
    let g = frame((6048, 4024), Rect::new(12, 12, 6000, 4000), 32767);
    let table = [29, 0, -36, -65, -101, -137, -165, -165, -115, 50, 359, 0, 0, 0, 0, 0];
    assert!(matches!(general_conversion(&table, 11, &g), Some(Opcode::WarpRectilinear2 { planes, .. }) if planes.len() == 1));
}

fn fixture(model: &'static str, table: &[i16; 16], count: u8, setting: u16) -> Arw {
    let mut arw = Arw::new(model, Little);
    arw.plain = plain(table, setting);
    set(&mut arw.plain, DISTORTION.table.0, sshort(i16::from(count), table));
    let mut sr2 = encrypted(table, setting);
    sr2.iter_mut().find(|(tag, _, _)| *tag == DISTORTION.table.1).unwrap().2[0] = u16::from(count);
    arw.sr2 = Some(sr2);
    arw
}

fn run(arw: &Arw, geometry: SonyLensGeometry) -> (SonyLensCorrections, Option<Opcode>, Option<Opcode>) {
    let bytes = arw.build();
    let tiff = Tiff::parse(&bytes).unwrap();
    let sr2 = SubIfd::read(&bytes, &tiff.ifds[0], Little);
    let (record, warp, lens) = corrections_and_off_warp(&bytes, Little, (&tiff.ifds[0], false), sr2.as_ref(), arw.model, geometry);
    (record.unwrap(), warp, lens)
}

/// No body is named: the same file gets the same lens warp under any model, whatever the camera's menu said, and
/// it stays out of the opcodes, which only validated conversions enter.
#[test]
fn any_body_gets_the_lens_warp_beside_the_opcodes() {
    let g = frame((6048, 4024), Rect::new(12, 12, 6000, 4000), 32767);
    let table = control([0; 16], "minus", 16);
    let expected = general_conversion(&table, 16, &g);
    assert!(expected.is_some());
    for model in ["ILCE-9M2", "ILCE-7RM3", "A body that does not exist yet"] {
        for setting in [0, 1, 17] {
            let (record, warp, lens) = run(&fixture(model, &table, 16, setting), g.clone());
            assert_eq!(warp, None, "{model} {setting}");
            assert_eq!(lens, expected, "{model} {setting}");
            assert_eq!(record.distortion.status, SonyLensStatus::NotValidatedModel);
        }
        // "no correction parameters available": nothing to trust
        assert_eq!(run(&fixture(model, &table, 16, 255), g.clone()).2, None);
    }
}

/// The earlier bodies write their only copy in the encrypted `SR2SubIFD`; a damaged raw-IFD copy is never replaced.
#[test]
fn a_lone_sr2_copy_is_used_and_disagreeing_copies_are_not() {
    let g = frame((4928, 3276), Rect::new(0, 0, 4912, 3264), 32767);
    let table = control([0; 16], "plus", 11);
    let mut lone = fixture("NEX-5N", &table, 11, 1);
    lone.plain.retain(|(tag, _)| ![DISTORTION.table.0, DISTORTION.setting.0].contains(tag));
    let (record, _, lens) = run(&lone, g.clone());
    assert!(accepted_copy(&record.distortion.table) && accepted_copy(&record.distortion.setting));
    assert_eq!(lens, general_conversion(&table, 11, &g));
    assert!(lens.is_some());

    let mut differing = fixture("NEX-5N", &table, 11, 1);
    let mut other = table;
    other[5] += 1;
    set(&mut differing.plain, DISTORTION.table.0, sshort(11, &other));
    let (record, _, lens) = run(&differing, g);
    assert!(!accepted_copy(&record.distortion.table));
    assert_eq!(lens, None);
}
