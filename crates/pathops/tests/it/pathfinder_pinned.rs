// Carved from VectorCraft's crates/pathops (tests/it/pathfinder_pinned.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

//! The exact output of every Pathfinder operation, pinned so that work on the windings and the
//! coverage tests can't change it: overlapping ellipses, rectangles and stars (even-odd and
//! non-zero, repeated keys for Merge) at a few counts.
//!
//! The pins hash the `Debug` text of each result. They were first taken from VectorCraft's own
//! `vectorcraft-pathops` at the commit named above: every result there, converted to a `BezPath`,
//! equals the one here, so the hashes below are of the same geometry (the text differs from
//! VectorCraft's, so VectorCraft's hashes could not be kept).

use craft_pathops::{FillRule, PathfinderOp, Shape, pathfinder};
use kurbo::{BezPath, Point, Rect};

use crate::common::{ellipse, polyline, rectangle};

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
                0 => ellipse(Rect::from_center_size(Point::new(x, y), (2.0 * s, 1.4 * s))),
                1 => rectangle(Rect::new(x, y, x + 1.5 * s, y + s)),
                _ => star(Point::new(x, y), s, s / 2.0),
            };
            let rule = if i % 4 == 3 { FillRule::EvenOdd } else { FillRule::NonZero };
            Shape::new(path, rule, (i % 5) as u64)
        })
        .collect()
}

/// Builds a five-pointed star (a rotation of 0.3°), with the cosines and sines read from
/// `STAR_UNIT_VECTORS`.
fn star(center: Point, r1: f64, r2: f64) -> BezPath {
    let pts: Vec<Point> = STAR_UNIT_VECTORS
        .iter()
        .enumerate()
        .map(|(i, &(cos, sin))| {
            let r = if i % 2 == 0 { r1 } else { r2 };
            Point::new(center.x + r * f64::from_bits(cos), center.y + r * f64::from_bits(sin))
        })
        .collect();
    polyline(&pts, true)
}

/// Each row holds the correctly rounded cosine and sine, as bits, of one of the 10 angles of a star
/// of 5 points and a rotation of 0.3°. A platform's libm can differ from them in the last bit.
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

/// Computed by this crate and checked equal, as geometry, to VectorCraft's results at the commit above.
const PINNED: [u64; 40] = [
    0x0c1c_c6ee_17c6_b93d,
    0x50fb_04dc_e546_55f0,
    0x35b2_1bb0_17ac_0f66,
    0x4969_5259_6b4a_d88a,
    0x8481_d1ef_6fab_e316,
    0x9a9f_cc86_e2ac_0794,
    0x53e5_e5d4_5f68_d14f,
    0x53e5_e5d4_5f68_d14f,
    0xc0a9_17f3_b4ed_06d7,
    0xfad9_1141_62ba_7ad2,
    0x6701_fae2_5165_2731,
    0x7495_2c22_3094_c615,
    0x5aea_a036_4d37_e326,
    0xcbf2_9ce4_8422_2325,
    0xab4c_3e5a_30b3_1e41,
    0xe251_6217_4e5d_cd1a,
    0x0807_20d0_69cb_3d50,
    0x0807_20d0_69cb_3d50,
    0xcbf2_9ce4_8422_2325,
    0x32c8_0cb7_8d98_eef3,
    0xdbfc_15ae_8690_d66d,
    0xbc12_47c2_1668_dc84,
    0xa1b4_76b2_4300_5abc,
    0xcbf2_9ce4_8422_2325,
    0xbab8_bd54_50c0_3e27,
    0x75ff_a2bb_723b_a92f,
    0xc9f6_3bb9_59e2_3ee1,
    0x7ebc_24e3_e02b_b0f1,
    0xec0d_7626_f06c_1df9,
    0xb2f0_bf41_9f77_4b24,
    0x00bb_7886_1501_2a2c,
    0xcbf2_9ce4_8422_2325,
    0x9ce1_4919_a124_952b,
    0xcbf2_9ce4_8422_2325,
    0x1967_b357_3407_b64a,
    0xfde9_8557_6b59_57b1,
    0xfcd1_239a_a416_4e7b,
    0x7ba5_f3dc_06cd_e11f,
    0x6ae0_c7a5_6cce_4b6a,
    0x3abd_47e2_98cf_8f69,
];
