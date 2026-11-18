//! The exact output of every Pathfinder operation, pinned so that work on the windings and the
//! coverage tests can't change it: overlapping ellipses, rectangles and stars (even-odd and
//! non-zero, repeated keys for Merge) at a few counts.

use craft_geom::{FillRule, PathData, Point, Rect, SubPath, shapes};
use craft_pathops::{PathfinderOp, Shape, pathfinder};

fn shapes_of(n: usize, seed: u64) -> Vec<Shape> {
    let mut r = seed | 1;
    let mut next = || {
        r ^= r << 13;
        r ^= r >> 7;
        r ^= r << 17;
        (r % 1_000_000) as f64 / 1_000_000.0
    };
    (0..n)
        .map(|i| {
            let (x, y, s) = (next() * 200.0, next() * 150.0, 15.0 + next() * 40.0);
            let path = match i % 3 {
                0 => shapes::ellipse(Rect::from_center_size(Point::new(x, y), (2.0 * s, 1.4 * s))),
                1 => shapes::rectangle(Rect::new(x, y, x + 1.5 * s, y + s)),
                _ => star(Point::new(x, y), s, s / 2.0),
            };
            let rule = if i % 4 == 3 { FillRule::EvenOdd } else { FillRule::NonZero };
            Shape::new(path, rule, (i % 5) as u64)
        })
        .collect()
}

/// Builds a star as `shapes::star(center, r1, r2, 5, 0.3)` does, with the cosines and sines read
/// from `STAR_UNIT_VECTORS`.
fn star(center: Point, r1: f64, r2: f64) -> PathData {
    let pts: Vec<Point> = STAR_UNIT_VECTORS
        .iter()
        .enumerate()
        .map(|(i, &(cos, sin))| {
            let r = if i % 2 == 0 { r1 } else { r2 };
            Point::new(center.x + r * f64::from_bits(cos), center.y + r * f64::from_bits(sin))
        })
        .collect();
    PathData::single(SubPath::polyline(&pts, true))
}

/// Each row holds the correctly rounded cosine and sine, as bits, of one of the 10 angles that
/// `shapes::star` computes for 5 points and a rotation of 0.3°. A platform's libm can differ from
/// them in the last bit.
const STAR_UNIT_VECTORS: [(u64, u64); 10] = [
    (0x3f75_724e_56b4_2a6c, 0xbfef_ffe3_40b4_d4f6),
    (0x3fe2_f1c5_9f37_cbf7, 0xbfe9_ca2a_1be5_b900),
    (0x3fee_7c33_ee44_9408, 0xbfd3_7547_1564_f9f7),
    (0x3fee_61b1_8a0e_c1cb, 0x3fd4_1873_d064_86bd),
    (0x3fe2_ac5e_9e54_9166, 0x3fe9_fc96_97b3_dc51),
    (0xbf75_724e_56b4_29df, 0x3fef_ffe3_40b4_d4f6),
    (0xbfe2_f1c5_9f37_cbf5, 0x3fe9_ca2a_1be5_b901),
    (0xbfee_7c33_ee44_9407, 0x3fd3_7547_1564_f9fd),
    (0xbfee_61b1_8a0e_c1cc, 0xbfd4_1873_d064_86b7),
    (0xbfe2_ac5e_9e54_9169, 0xbfe9_fc96_97b3_dc4f),
];

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

#[test]
fn every_pathfinder_operation_is_pinned() {
    use PathfinderOp::*;
    let ops = [Unite, MinusFront, MinusBack, Intersect, Exclude, Divide, Trim, Merge, Crop, Outline];
    let mut got = vec![];
    for (n, seed) in [(2, 3), (5, 7), (12, 11), (30, 13)] {
        let input = shapes_of(n, seed);
        for op in ops {
            let out = pathfinder(op, &input);
            let text: String = out.iter().map(|s| format!("{}|{:?}|{:?};", s.key, s.rule, s.path)).collect();
            got.push(fnv(&text));
        }
    }
    assert_eq!(got.len(), PINNED.len());
    assert_eq!(got, PINNED);
}

/// Computed with the dense windings (main at b08482d6).
const PINNED: [u64; 40] = [
    0x5e62_baae_d8b4_dba5,
    0x2bbc_e138_a97c_5a28,
    0xce78_992e_750b_1092,
    0x30cf_cc0b_861c_48b4,
    0xe6e4_2a29_1e16_5e86,
    0x390b_0633_5066_6292,
    0x06b7_3884_f961_c343,
    0x06b7_3884_f961_c343,
    0x0384_d599_5a03_dafd,
    0x3dcd_029a_dd3f_fa38,
    0x8249_71f8_fda4_2bb6,
    0xe7f2_8633_cef9_a76c,
    0x1a11_084e_721e_ac3b,
    0xcbf2_9ce4_8422_2325,
    0xa786_d973_a0d7_ca2a,
    0x389b_c89a_0ccf_578f,
    0xff9d_cf53_d872_681e,
    0xff9d_cf53_d872_681e,
    0xcbf2_9ce4_8422_2325,
    0x6be3_02a6_d9c9_e80e,
    0x3525_cabe_b235_aaef,
    0xf969_aa07_a44d_1770,
    0xed13_60c7_0aaa_7854,
    0xcbf2_9ce4_8422_2325,
    0x6550_a381_db6b_a35f,
    0x1ba9_6dfb_89ea_ac78,
    0x1344_b467_dd20_5f02,
    0x948f_e2a1_d657_0d4c,
    0x49a4_5c3c_2d96_14af,
    0x16a7_ce9e_4a3c_0974,
    0x0b12_c643_a446_da04,
    0xcbf2_9ce4_8422_2325,
    0xeb6a_6614_cea5_a508,
    0xcbf2_9ce4_8422_2325,
    0xb9c4_2803_7db9_bd4f,
    0x34b6_90d5_e443_b799,
    0xdfaf_ecbd_2d58_3227,
    0x6d5e_6e8d_b05b_8688,
    0x59dc_9268_a73f_e92e,
    0x5a83_5cbd_1f8c_5143,
];
