use super::*;

fn geometry(mode: &str) -> SonyLensGeometry {
    let (sw, sh, x, y, w, h, compression, linear_ycbcr) = match mode {
        "a1-full-u" => (8672, 5784, 12, 12, 8640, 5760, 1, false),
        "a1-full-l" => (8704, 6144, 12, 12, 8640, 5760, 7, false),
        "a1-apsc-c" => (5664, 3768, 12, 12, 5616, 3744, 32767, false),
        "a1-7834" => (4608, 3072, 8, 8, 4320, 2880, 7, true),
        _ => panic!("unknown mode {mode}"),
    };
    let crop = Rect::new(x, y, w, h);
    SonyLensGeometry {
        storage_width: sw,
        storage_height: sh,
        sony_crop: Some(crop),
        default_crop: Some(crop),
        image_crop: crop,
        reference: crop,
        compression,
        linear_ycbcr,
        native_3_2: true,
    }
}

/// Where the warp moves crop pixel (`x`, `y`), as a displacement in pixels.
fn displacement(op: &Opcode, g: &SonyLensGeometry, x: f64, y: f64) -> [f64; 2] {
    let (aw, ah) = (g.storage_width as f64 - 1.0, g.storage_height as f64 - 1.0);
    let (center, gain): ([f64; 2], Box<dyn Fn(f64) -> f64>) = match op {
        Opcode::WarpRectilinear { planes, center } => {
            assert_eq!(planes.len(), 1);
            let k = planes[0];
            (*center, Box::new(move |r: f64| k[0] + r * r * (k[1] + r * r * (k[2] + r * r * k[3]))))
        }
        Opcode::WarpRectilinear2 { planes, center, reciprocal, .. } => {
            assert_eq!(planes.len(), 1);
            let (k, reciprocal) = (planes[0], *reciprocal);
            (*center, Box::new(move |r: f64| crate::opcodes::wr2_factor(&k, reciprocal, r).unwrap()))
        }
        other => panic!("not a radial warp: {other:?}"),
    };
    let (cx, cy) = (center[0] * aw, center[1] * ah);
    let m = crate::opcodes::max_radius(cx, cy, aw, ah);
    let (dx, dy) = (x + g.image_crop.x as f64 - cx, y + g.image_crop.y as f64 - cy);
    let gain = gain(dx.hypot(dy) / m);
    [dx * (gain - 1.0), dy * (gain - 1.0)]
}

/// Sony's own converter, given containers that differ only in this record, moved 247 positions per control. The map
/// reproduces every one, and its warp every smooth curve, in full-frame (16 knots) and APS-C (11 knots) frames.
#[test]
fn the_map_and_its_warp_match_independent_sony_positions() {
    let controls: serde_json::Value =
        serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sony/sony-extended-natural-2026-11.json"))).unwrap();
    let rows = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sony/sony-extended-natural-2026-11.tsv"));
    let mut opcodes = std::collections::HashMap::new();
    let (mut tested, mut fitted) = (0, 0);
    for row in rows.lines().skip(1) {
        let v: Vec<_> = row.split('\t').collect();
        let count: u16 = v[4].parse().unwrap();
        let c = controls["controls"].as_array().unwrap().iter().find(|c| c["name"] == v[0] && c["variant"] == v[1]).unwrap();
        let table: [i16; 19] = serde_json::from_value(c["table"].clone()).unwrap();
        let g = geometry(v[0]);
        let (w, h) = (g.image_crop.width as f64, g.image_crop.height as f64);
        let numbers: Vec<f64> = v[5..].iter().map(|v| v.parse().unwrap()).collect();
        let (dx, dy) = (numbers[0] - (w - 1.0) / 2.0, numbers[1] - (h - 1.0) / 2.0);
        let r = dx.hypot(dy) / w.hypot(h) * 2.0;
        let map = Map::new(&table, count, w / h).unwrap();
        let gain = if r == 0.0 { 1.0 } else { map.source(r) / r };
        let error = (dx * (gain - 1.0) - numbers[2]).hypot(dy * (gain - 1.0) - numbers[3]) * 1440.0 / w;
        assert!(error < 0.38, "exact {} {}: {error}", v[0], v[1]);
        tested += 1;
        // Localized bumps establish the knot spacing but are intentionally too sharp for a polynomial warp.
        if !v[1].contains("bump") {
            let op = opcodes.entry((v[0], v[1])).or_insert_with(|| general_opcode(&table, count, &g).expect("a natural curve must fit"));
            let predicted = displacement(op, &g, numbers[0], numbers[1]);
            let error = (predicted[0] - numbers[2]).hypot(predicted[1] - numbers[3]) * 1440.0 / w;
            assert!(error < 0.6, "warp {} {}: {error}", v[0], v[1]);
            fitted += 1;
        }
    }
    assert_eq!((tested, fitted), (18 * 247, 12 * 247));
}

#[test]
fn the_warp_matches_original_camera_door_edges() {
    let g = geometry("a1-full-l");
    let table = [0, -1, -4, -7, -12, -18, -26, -34, -37, -32, -9, 35, 113, 235, 419, 678, 1028, 1486, 2067];
    let op = general_opcode(&table, 16, &g).unwrap();
    let rows: Vec<Vec<f64>> = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sony/sony-a1ii-lossless-door-2026-11.tsv"))
        .lines()
        .skip(1)
        .map(|l| l.split('\t').map(|v| v.parse().unwrap()).collect())
        .collect();
    let mut errors = Vec::new();
    for row in &rows {
        for side in 0..2 {
            let d = displacement(&op, &g, row[3 + side], row[0]);
            let (x, y) = (row[3 + side] + d[0], row[0] + d[1]);
            let i = rows.partition_point(|r| r[0] < y).saturating_sub(1).min(rows.len() - 2);
            let (a, b) = (&rows[i], &rows[i + 1]);
            let expected = a[side + 1] + (b[side + 1] - a[side + 1]) * (y - a[0]) / (b[0] - a[0]);
            errors.push((x - expected).abs() / 6.0);
        }
    }
    errors.sort_by(f64::total_cmp);
    assert_eq!(errors.len(), 280);
    assert!(errors[140] < 0.15 && errors[279] < 0.4, "median {} max {}", errors[140], errors[279]);
}

#[test]
fn the_warp_refuses_other_frames_folds_and_inaccurate_fits() {
    let g = geometry("a1-full-u");
    let table = std::array::from_fn(|i| (i * i * 4) as i16);
    assert!(general_opcode(&table, 16, &g).is_some());
    for mutation in 0..3 {
        let mut bad = g.clone();
        match mutation {
            // an in-camera crop mode: two thirds of the sensor inside the stored frame
            0 => (bad.image_crop, bad.reference) = (Rect::new(1464, 972, 5760, 3840), Rect::new(1464, 972, 5760, 3840)),
            // a reference that leaves the stored image
            1 => bad.reference.x += 40,
            // an aspect crop whose reference is not 3:2 either, with Sony's tags present
            _ => bad.reference.height -= 600,
        }
        assert!(general_opcode(&table, 16, &bad).is_none(), "layout mutation {mutation}");
    }
    for mutation in 0..4 {
        let mut bad = table;
        match mutation {
            0 => bad[0] = 1,
            1 => bad[18] = MAX_WORD + 1,
            2 => {
                bad = [0; 19];
                bad[6] = -192;
            }
            _ => {
                bad = [0; 19];
                bad[6] = -4200;
            }
        }
        assert!(general_opcode(&bad, 16, &g).is_none(), "table mutation {mutation}");
    }
    assert!(general_opcode(&table, 12, &g).is_none());
}

/// Lossless files pad the stored frame; the same normalized map must give the same picture.
#[test]
fn storage_padding_does_not_change_the_optical_mapping() {
    let table = [0, -31, -123, -274, -478, -728, -1015, -1333, -1672, -2025, -2384, -2741, -3088, -3416, -3711, -3954, -4120, -4183, -4118];
    let radial: Vec<f64> = ["a1-full-u", "a1-full-l"]
        .iter()
        .map(|mode| {
            let g = geometry(mode);
            let op = general_opcode(&table, 16, &g).expect("a strong natural barrel must fit despite storage padding");
            let (w, h) = (g.image_crop.width as f64, g.image_crop.height as f64);
            displacement(&op, &g, (w - 1.0) / 2.0 + w * 0.4, (h - 1.0) / 2.0 + h * 0.4)[0] / w
        })
        .collect();
    assert!((radial[0] - radial[1]).abs() < 1e-12);
}

/// A one-pixel stored frame has no extent to normalise the centre by: no opcode, never a NaN centre.
#[test]
fn a_single_pixel_frame_gets_no_warp() {
    let mut g = geometry("a1-full-u");
    let crop = Rect::new(0, 0, 1, 1);
    (g.storage_width, g.storage_height) = (1, 1);
    (g.sony_crop, g.default_crop, g.image_crop, g.reference) = (Some(crop), Some(crop), crop, crop);
    let table = std::array::from_fn(|i| (i * i * 4) as i16);
    let map = Map::new(&table, 16, 1.0).unwrap();
    if let Some(Opcode::WarpRectilinear { center, .. }) = fit_opcode(&map, &g, crop) {
        panic!("centre {center:?}");
    }
    assert!(general_opcode(&table, 16, &g).is_none());
}
