use craft_cms::sony_lut::{MAX_SIDE, parse_spi3d, parse_spimtx};

#[test]
fn spimtx_is_finite_exact_and_scales_offsets() {
    let m = parse_spimtx("2 0 0 6553.5\n0 3 0 -65535\n0 0 4 0").unwrap();
    assert_eq!(m.matrix, [2.0, 0.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 0.0, 4.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
    assert_eq!(m.offset, [0.1, -1.0, 0.0, 0.0]);
    for bad in ["", "1 0 0 0", "NaN 0 0 0 0 1 0 0 0 0 1 0", "inf 0 0 0 0 1 0 0 0 0 1 0", "1 0 0 0 0 1 0 0 0 0 1 0 9"] {
        assert!(parse_spimtx(bad).is_none());
    }
}

fn table() -> String {
    let mut text = String::from("SPILUT 1.0\n3 3\n2 2 2\n");
    // Reverse row order to prove coordinates, rather than row order, determine storage.
    for i in (0..8).rev() {
        let (r, g, b) = (i & 1, (i >> 1) & 1, i >> 2);
        text.push_str(&format!("{r} {g} {b} {r} {g} {b}\n"));
    }
    text
}

#[test]
fn spi3d_red_fast_coordinates_and_limits() {
    let text = table();
    let lut = parse_spi3d(&text).unwrap();
    assert_eq!(lut.size, 2);
    for (i, row) in lut.values.iter().enumerate() {
        assert_eq!(*row, [(i & 1) as f32, ((i >> 1) & 1) as f32, (i >> 2) as f32]);
    }
    assert!(parse_spi3d(&format!("{text}0 0 0 0 0 0\n")).is_none());
    assert!(parse_spi3d("SPILUT 1.0\n3 3\n2 2 2\n0 0 0 0 0 0\n").is_none());
    assert!(parse_spi3d(&format!("SPILUT 1.0\n3 3\n{0} {0} {0}\n", MAX_SIDE + 1)).is_none());
    assert!(parse_spi3d(&text.replace("2 2 2", "2 2 3")).is_none());
    assert!(parse_spi3d(&text.replace("0 0 0 0 0 0", "2 0 0 0 0 0")).is_none());
    assert!(parse_spi3d(&text.replace("0 0 0 0 0 0", "0 0 0 0 0 0 extra")).is_none());
}

#[test]
fn inherited_tolerance_is_explicit() {
    let text = "SPILUT alternative\nignored\n2 2 2\n".to_owned() + &"0 0 0 -1 2 NaN\n".repeat(8);
    let lut = parse_spi3d(&text).unwrap();
    assert_eq!(&lut.values[0][..2], &[-1.0, 2.0]);
    assert!(lut.values[0][2].is_nan());
    assert_eq!(lut.values[1], [0.0; 3]);
}
