//! Synthetic frames written bit by bit, checked against values observed from ffmpeg for the same
//! constructions (see `tools/bink_probe`), and hostile input.

use super::*;

/// Writes bits least significant first, as the format reads them.
#[derive(Default)]
struct BitWriter {
    bits: Vec<bool>,
}

impl BitWriter {
    fn put(&mut self, value: u32, n: u32) -> &mut BitWriter {
        self.bits.extend((0..n).map(|k| value >> k & 1 != 0));
        self
    }

    fn align32(&mut self) -> &mut BitWriter {
        while !self.bits.len().is_multiple_of(32) {
            self.bits.push(false);
        }
        self
    }

    fn bytes(&self) -> Vec<u8> {
        self.bits.chunks(8).map(|c| c.iter().enumerate().fold(0u8, |b, (k, &v)| b | u8::from(v) << k)).collect()
    }
}

/// Width of a bundle's length field for `n` values per row.
fn length_bits(n: u32) -> u32 {
    32 - (n + 511).leading_zeros()
}

/// One row of blocks of a plane `bw` blocks wide, every tree raw. Each bundle is one chunk.
#[derive(Default)]
struct Row {
    types: Vec<u32>,
    sub_types: Vec<u32>,
    colours: Vec<u32>,
    patterns: Vec<u32>,
    /// (magnitude, negative)
    x_off: Vec<(u32, bool)>,
    y_off: Vec<(u32, bool)>,
    /// One intra DC value, if any.
    intra_dc: Option<u32>,
    /// One inter DC value as (magnitude, negative), if any.
    inter_dc: Option<(u32, bool)>,
    runs: Vec<u32>,
    /// The blocks' in-place bits, in order.
    inline: Vec<(u32, u32)>,
}

impl Row {
    fn write(&self, w: &mut BitWriter, bw: u32) {
        for _ in 0..23 {
            w.put(0, 4); // tree 0: raw nibbles
        }
        let raw = |w: &mut BitWriter, values: &[u32], n: u32| {
            w.put(values.len() as u32, length_bits(n));
            if !values.is_empty() {
                w.put(0, 1);
                for &v in values {
                    w.put(v, 4);
                }
            }
        };
        raw(w, &self.types, bw);
        raw(w, &self.sub_types, bw / 2);
        w.put(self.colours.len() as u32, length_bits(bw * 64));
        if !self.colours.is_empty() {
            w.put(0, 1);
            for &c in &self.colours {
                w.put(c >> 4, 4).put(c & 15, 4);
            }
        }
        w.put(self.patterns.len() as u32, length_bits(bw * 8));
        for &p in &self.patterns {
            w.put(p & 15, 4).put(p >> 4, 4);
        }
        for offsets in [&self.x_off, &self.y_off] {
            w.put(offsets.len() as u32, length_bits(bw));
            if !offsets.is_empty() {
                w.put(0, 1);
                for &(magnitude, negative) in offsets {
                    w.put(magnitude, 4);
                    if magnitude != 0 {
                        w.put(u32::from(negative), 1);
                    }
                }
            }
        }
        w.put(u32::from(self.intra_dc.is_some()), length_bits(bw));
        if let Some(dc) = self.intra_dc {
            w.put(dc, 11);
        }
        w.put(u32::from(self.inter_dc.is_some()), length_bits(bw));
        if let Some((magnitude, negative)) = self.inter_dc {
            w.put(magnitude, 10);
            if magnitude != 0 {
                w.put(u32::from(negative), 1);
            }
        }
        raw(w, &self.runs, bw * 48);
        for &(value, n) in &self.inline {
            w.put(value, n);
        }
        w.align32();
    }
}

/// A flat chroma plane one block wide.
fn flat_chroma(value: u32) -> Row {
    Row { types: vec![6], colours: vec![value], ..Row::default() }
}

/// A revision `i` packet for a 16x8 picture: one row of two luma blocks, one block per chroma plane.
fn packet(luma: &Row, chroma: &Row) -> Vec<u8> {
    let mut w = BitWriter::default();
    w.put(0, 32);
    luma.write(&mut w, 2);
    chroma.write(&mut w, 1);
    chroma.write(&mut w, 1);
    w.bytes()
}

fn decoder() -> Decoder {
    Decoder::new(16, 8, *b"BIKi", 0).unwrap()
}

/// The 8x8 luma block at block column `bx`, as rows.
fn block(pic: &Picture, bx: usize) -> Vec<Vec<u8>> {
    pic.y.chunks(16).map(|row| row[bx * 8..bx * 8 + 8].to_vec()).collect()
}

#[test]
fn refuses_what_it_cannot_decode() {
    assert!(matches!(Decoder::new(16, 16, *b"KB2g", 0), Err(Error::Invalid(_))));
    assert!(matches!(Decoder::new(16, 16, *b"BIKb", 0), Err(Error::Unsupported(_))));
    assert!(matches!(Decoder::new(16, 16, *b"BIKi", FLAG_ALPHA), Err(Error::Unsupported(_))));
    assert!(matches!(Decoder::new(16, 16, *b"BIKi", FLAG_GRAYSCALE), Err(Error::Unsupported(_))));
    assert!(matches!(Decoder::new(0, 16, *b"BIKi", 0), Err(Error::Invalid(_))));
    assert!(matches!(Decoder::new(16, 32768, *b"BIKi", 0), Err(Error::Invalid(_))));
    assert!(Decoder::new(16, 16, *b"BIKi", 0).is_ok());
    assert!(Decoder::new(16, 16, *b"BIKf", 0).is_ok());
}

#[test]
fn decoder_from_container_parameters() {
    let params =
        craft_media::CodecParams::new(craft_media::CodecId::BinkVideo).with_tag(*b"BIKi").with_size(16, 8).with_extradata(0u32.to_le_bytes());
    assert!(Decoder::from_params(&params).is_ok());
    let alpha = params.clone().with_extradata(FLAG_ALPHA.to_le_bytes());
    assert!(matches!(Decoder::from_params(&alpha), Err(Error::Unsupported(_))));
    let no_size = craft_media::CodecParams::new(craft_media::CodecId::BinkVideo).with_tag(*b"BIKi");
    assert!(matches!(Decoder::from_params(&no_size), Err(Error::Invalid(_))));
}

#[test]
fn fill_raw_and_chroma_planes() {
    let raw: Vec<u32> = (1..=64).collect();
    let mut colours = vec![0x70];
    colours.extend(&raw);
    let luma = Row { types: vec![6, 9], colours, ..Row::default() };
    let pic = decoder().decode(&packet(&luma, &flat_chroma(0x55))).unwrap();
    assert_eq!((pic.width, pic.height, pic.chroma_width, pic.chroma_height), (16, 8, 8, 4));
    assert!(block(&pic, 0).iter().flatten().all(|&v| v == 0x70));
    assert_eq!(block(&pic, 1).concat(), (1..=64).collect::<Vec<u8>>());
    // revision i stores Cr before Cb; both are 0x55 here
    assert!(pic.u.iter().chain(&pic.v).all(|&v| v == 0x55));
    assert_eq!((pic.u.len(), pic.v.len()), (32, 32));
}

#[test]
fn run_fill_follows_the_scan_pattern() {
    // scan 0 runs down the first column, up the second, and so on; 64 runs of one sample
    let mut inline = vec![(0, 4)];
    inline.extend(std::iter::repeat_n((0, 1), 64));
    let mut colours: Vec<u32> = (1..=64).collect();
    colours.push(200);
    let luma = Row { types: vec![3, 6], colours, runs: vec![0; 64], inline, ..Row::default() };
    let pic = decoder().decode(&packet(&luma, &flat_chroma(128))).unwrap();
    let b = block(&pic, 0);
    assert_eq!(b[0], [1, 16, 17, 32, 33, 48, 49, 64]);
    assert_eq!(b[7], [8, 9, 24, 25, 40, 41, 56, 57]);
    assert!(block(&pic, 1).iter().flatten().all(|&v| v == 200));

    // 16 runs of four; the flag alternates between one colour for the run and a colour each
    let mut inline = vec![(0, 4)];
    inline.extend((0..16).map(|i| (u32::from(i % 2 == 0), 1)));
    let luma = Row { types: vec![3, 6], colours: (1..=41).collect(), runs: vec![3; 16], inline, ..Row::default() };
    let pic = decoder().decode(&packet(&luma, &flat_chroma(128))).unwrap();
    let column0: Vec<u8> = block(&pic, 0).iter().map(|r| r[0]).collect();
    assert_eq!(column0, [1, 1, 1, 1, 2, 3, 4, 5]);
    assert!(block(&pic, 1).iter().flatten().all(|&v| v == 41));
}

#[test]
fn pattern_fill_picks_the_second_colour_by_bit() {
    let luma = Row { types: vec![8, 6], colours: vec![10, 20, 30], patterns: vec![0b0000_0101; 8], ..Row::default() };
    let pic = decoder().decode(&packet(&luma, &flat_chroma(128))).unwrap();
    assert!(block(&pic, 0).iter().all(|r| r == &[20, 10, 20, 10, 10, 10, 10, 10]));
}

#[test]
fn intra_dct_block_levels_match_ffmpeg() {
    // no coefficients: plane count 0, then the quantiser index. DC 800 gives 100 at quantiser 0
    // and 133 at quantiser 1 (scale 4/3)
    for (quantiser, level) in [(0, 100), (1, 133), (2, 167)] {
        let luma = Row { types: vec![5, 6], colours: vec![1], intra_dc: Some(800), inline: vec![(0, 4), (quantiser, 4)], ..Row::default() };
        let pic = decoder().decode(&packet(&luma, &flat_chroma(128))).unwrap();
        assert!(block(&pic, 0).iter().flatten().all(|&v| v == level), "quantiser {quantiser}");
    }
    // rounding of a lone DC value: 4 -> 0, 7 -> 1, 15 -> 2, and no clamping: 2047 -> 0
    for (dc, level) in [(4, 0), (7, 1), (15, 2), (2040, 255), (2047, 0)] {
        let luma = Row { types: vec![5, 6], colours: vec![1], intra_dc: Some(dc), inline: vec![(0, 4), (0, 4)], ..Row::default() };
        let pic = decoder().decode(&packet(&luma, &flat_chroma(128))).unwrap();
        assert!(block(&pic, 0).iter().flatten().all(|&v| v == level), "DC {dc}");
    }
}

#[test]
fn a_single_coefficient_gives_the_basis_pattern_ffmpeg_gives() {
    // one plane-count nibble of 9 (values up to 511), coefficient (0, 1) = 400: the walk flags
    // the single position (0, 1) in the top plane. The list is [group, group, group, (0,1), ...].
    // 400 = 0b1_1001_0000: top plane 8, low bits 0b1001_0000.
    let mut inline = vec![(9, 4)];
    inline.extend([(0, 1), (0, 1), (0, 1), (1, 1), (0b1001_0000, 8), (0, 1)]); // three groups, then (0,1) with low bits and sign
    inline.extend([(0, 1), (0, 1)]); // (1,0) and (1,1) in plane 8
    // planes 7..0: three groups and two singles each
    inline.extend(std::iter::repeat_n((0, 5), 8));
    inline.push((0, 4)); // quantiser 0
    let luma = Row { types: vec![5, 6], colours: vec![1], intra_dc: Some(1024), inline, ..Row::default() };
    let pic = decoder().decode(&packet(&luma, &flat_chroma(128))).unwrap();
    let expect: Vec<u8> = [69, 59, 39, 14, -14, -39, -59, -69].iter().map(|d: &i32| (128 + d) as u8).collect();
    assert!(block(&pic, 0).iter().all(|r| r == &expect), "{:?}", block(&pic, 0)[0]);
}

#[test]
fn inter_blocks_build_on_the_previous_picture() {
    let mut dec = decoder();
    let raw: Vec<u32> = (0..64).map(|i| 40 + i).collect();
    let mut colours = raw.clone();
    colours.push(90);
    let first = Row { types: vec![9, 6], colours, ..Row::default() };
    let chroma_skip = Row { types: vec![0], ..Row::default() };
    dec.decode(&packet(&first, &flat_chroma(128))).unwrap();

    // skip, then a motion block that copies the block 8 samples to its left
    let second = Row { types: vec![0, 2], x_off: vec![(8, true)], y_off: vec![(0, false)], ..Row::default() };
    let pic = dec.decode(&packet(&second, &chroma_skip)).unwrap();
    assert_eq!(block(&pic, 0), block(&pic, 1));
    assert_eq!(block(&pic, 1).concat(), (40..104).collect::<Vec<u8>>());
    assert!(pic.u.iter().all(|&v| v == 128));

    // inter DCT with no coefficients: DC +80 adds 10, DC -80 at quantiser 3 subtracts 20
    for (dc, quantiser, add) in [((80, false), 0, 10i32), ((80, true), 3, -20)] {
        let third = Row {
            types: vec![7, 0],
            x_off: vec![(0, false)],
            y_off: vec![(0, false)],
            inter_dc: Some(dc),
            inline: vec![(0, 4), (quantiser, 4)],
            ..Row::default()
        };
        let before = block(&pic, 0).concat();
        let mut d = decoder();
        d.decode(&packet(&first, &flat_chroma(128))).unwrap();
        let after = d.decode(&packet(&third, &chroma_skip)).unwrap();
        let want: Vec<u8> = before.iter().map(|&v| (i32::from(v) + add) as u8).collect();
        assert_eq!(block(&after, 0).concat(), want);
    }

    // residue: budget 1, top plane 0, the first group flagged, its first member non-zero and positive
    let fourth = Row {
        types: vec![4, 0],
        x_off: vec![(0, false)],
        y_off: vec![(0, false)],
        inline: vec![(0, 7), (0, 3), (1, 1), (0, 1), (0, 1)],
        ..Row::default()
    };
    let mut d = decoder();
    let base = d.decode(&packet(&first, &flat_chroma(128))).unwrap();
    let after = d.decode(&packet(&fourth, &chroma_skip)).unwrap();
    let mut want = block(&base, 0);
    want[0][2] += 1;
    assert_eq!(block(&after, 0), want);
}

#[test]
fn a_motion_vector_outside_the_plane_is_an_error() {
    let mut dec = decoder();
    let first = Row { types: vec![6, 6], colours: vec![1, 2], ..Row::default() };
    dec.decode(&packet(&first, &flat_chroma(128))).unwrap();
    let chroma_skip = Row { types: vec![0], ..Row::default() };
    for (x, y) in [((1, true), (0, false)), ((0, false), (1, false)), ((0, false), (1, true)), ((9, false), (0, false))] {
        let bad = Row { types: vec![2, 0], x_off: vec![x], y_off: vec![y], ..Row::default() };
        assert!(matches!(dec.decode(&packet(&bad, &chroma_skip)), Err(Error::Invalid(_))), "{x:?} {y:?}");
    }
    // the picture before the failed frames is still the reference
    let skip = Row { types: vec![0, 0], ..Row::default() };
    let pic = dec.decode(&packet(&skip, &chroma_skip)).unwrap();
    assert!(block(&pic, 0).iter().flatten().all(|&v| v == 1));
    assert!(block(&pic, 1).iter().flatten().all(|&v| v == 2));
}

#[test]
fn revision_f_colours_are_signed_and_its_chroma_order_differs() {
    let mut w = BitWriter::default();
    // no size word; luma, Cb, Cr
    Row { types: vec![6, 6], colours: vec![0xf0, 0x10], ..Row::default() }.write(&mut w, 2);
    flat_chroma(0x00).write(&mut w, 1);
    flat_chroma(0x90).write(&mut w, 1);
    let pic = Decoder::new(16, 8, *b"BIKf", 0).unwrap().decode(&w.bytes()).unwrap();
    assert!(block(&pic, 0).iter().flatten().all(|&v| v == 16));
    assert!(block(&pic, 1).iter().flatten().all(|&v| v == 144));
    assert!(pic.u.iter().all(|&v| v == 128));
    assert!(pic.v.iter().all(|&v| v == 112));

    // the same planes as revision i: unsigned colours, and the second plane is Cr
    let mut w = BitWriter::default();
    w.put(0, 32);
    Row { types: vec![6, 6], colours: vec![0xf0, 0x10], ..Row::default() }.write(&mut w, 2);
    flat_chroma(0x00).write(&mut w, 1);
    flat_chroma(0x90).write(&mut w, 1);
    let pic = decoder().decode(&w.bytes()).unwrap();
    assert!(block(&pic, 0).iter().flatten().all(|&v| v == 0xf0));
    assert!(pic.v.iter().all(|&v| v == 0x00));
    assert!(pic.u.iter().all(|&v| v == 0x90));
}

#[test]
fn truncated_and_empty_packets_are_errors() {
    let luma = Row { types: vec![6, 6], colours: vec![1, 2], ..Row::default() };
    let good = packet(&luma, &flat_chroma(128));
    let mut dec = decoder();
    assert!(matches!(dec.decode(&[]), Err(Error::Invalid(_))));
    assert!(matches!(dec.decode(&good[..3]), Err(Error::Invalid(_))));
    for cut in [4, 8, 16, 24, good.len() - 12] {
        assert!(dec.decode(&good[..cut]).is_err(), "cut at {cut}");
    }
    assert!(dec.decode(&good).is_ok());
}

#[test]
fn hostile_input_never_panics() {
    let raw: Vec<u32> = (1..=64).collect();
    let mut run_inline = vec![(3, 4)];
    run_inline.extend((0..16).map(|i| (u32::from(i % 2 == 0), 1)));
    let seeds = [
        packet(&Row { types: vec![6, 9], colours: std::iter::once(7).chain(raw.iter().copied()).collect(), ..Row::default() }, &flat_chroma(128)),
        packet(
            &Row { types: vec![3, 8], colours: (1..=43).collect(), patterns: vec![0x5a; 8], runs: vec![3; 16], inline: run_inline, ..Row::default() },
            &flat_chroma(1),
        ),
        packet(
            &Row {
                types: vec![5, 6],
                colours: vec![1],
                intra_dc: Some(800),
                inline: vec![(6, 4), (0x2f5a_91c3, 32), (0x0137_fe55, 32), (5, 4)],
                ..Row::default()
            },
            &flat_chroma(2),
        ),
        packet(
            &Row {
                types: vec![7, 4],
                x_off: vec![(0, false), (0, false)],
                y_off: vec![(0, false), (0, false)],
                inter_dc: Some((80, true)),
                inline: vec![(2, 4), (0x35, 12), (4, 4), (90, 7), (5, 3), (0x9e37_79b9, 32)],
                ..Row::default()
            },
            &Row { types: vec![0], ..Row::default() },
        ),
    ];
    for (i, seed) in seeds.iter().enumerate() {
        craft_media_testkit::hostile::hammer(&format!("bink video seed {i}"), seed, 20_000, |d| {
            for signature in [*b"BIKi", *b"BIKf"] {
                if let Ok(mut dec) = Decoder::new(16, 8, signature, 0) {
                    let _ = dec.decode(d);
                    let _ = dec.decode(d);
                }
            }
        });
    }
}
