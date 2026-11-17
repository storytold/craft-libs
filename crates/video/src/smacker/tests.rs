use super::*;
use craft_media_testkit::smacker::{BigTree as TreeWriter, BitWriter};

/// Escape values no test symbol uses, so the cache leaves stay out of the trees.
const NO_ESCAPES: [u16; 3] = [0xFFF0, 0xFFF1, 0xFFF2];

/// The four trees of a stream (maps, mono colours, full colours, types) and its parameters.
struct Stream {
    maps: TreeWriter,
    colours: TreeWriter,
    full: TreeWriter,
    types: TreeWriter,
}

impl Stream {
    fn new(maps: &[u16], colours: &[u16], full: &[u16], types: &[u16]) -> Stream {
        let t = |s: &[u16]| TreeWriter::new(s, NO_ESCAPES);
        Stream { maps: t(maps), colours: t(colours), full: t(full), types: t(types) }
    }

    fn params(&self, width: u32, height: u32, tag: &[u8; 4]) -> CodecParams {
        let trees = [&self.maps, &self.colours, &self.full, &self.types];
        let mut extradata: Vec<u8> = trees.iter().flat_map(|t| t.size().to_le_bytes()).collect();
        let mut w = BitWriter::new();
        for t in trees {
            t.write(&mut w);
        }
        extradata.extend(w.finish());
        CodecParams::new(CodecId::SmackerVideo).with_tag(*tag).with_size(width, height).with_extradata(extradata)
    }

    fn decoder(&self, width: u32, height: u32, tag: &[u8; 4]) -> Decoder {
        Decoder::new(&self.params(width, height, tag)).unwrap()
    }
}

/// A block type code: the type, the run index and the colour of a solid block.
const fn code(kind: u16, run: u16, colour: u16) -> u16 {
    kind | run << 2 | colour << 8
}

const MONO: u16 = 0;
const FULL: u16 = 1;
const VOID: u16 = 2;
const SOLID: u16 = 3;

fn rows(dec: &Decoder) -> Vec<&[u8]> {
    dec.pixels().chunks(dec.width() as usize).collect()
}

#[test]
fn solid_void_mono_and_full_blocks() {
    let types = [code(SOLID, 1, 7), code(VOID, 0, 0), code(MONO, 0, 0), code(FULL, 0, 0), code(SOLID, 0, 9)];
    let s = Stream::new(&[0xA5C3, 0x0001], &[0x2010], &[0x0201, 0x0403, 0x0605], &types);
    let mut dec = s.decoder(16, 4, b"SMK2");

    // two solid blocks of 7, a mono block, a full block
    let mut w = BitWriter::new();
    s.types.put(&mut w, code(SOLID, 1, 7));
    s.types.put(&mut w, code(MONO, 0, 0));
    s.colours.put(&mut w, 0x2010);
    s.maps.put(&mut w, 0xA5C3);
    s.types.put(&mut w, code(FULL, 0, 0));
    for pair in [0x0201, 0x0403, 0x0403, 0x0605, 0x0605, 0x0201, 0x0201, 0x0201] {
        s.full.put(&mut w, pair);
    }
    dec.decode(None, &w.finish()).unwrap();
    let r = rows(&dec);
    // mono: a set map bit takes the high colour byte; bit 0 is the top left pixel
    // full: each row is two codes, the right pair first
    assert_eq!(r[0], [7, 7, 7, 7, 7, 7, 7, 7, 0x20, 0x20, 0x10, 0x10, 3, 4, 1, 2]);
    assert_eq!(r[1], [7, 7, 7, 7, 7, 7, 7, 7, 0x10, 0x10, 0x20, 0x20, 5, 6, 3, 4]);
    assert_eq!(r[2], [7, 7, 7, 7, 7, 7, 7, 7, 0x20, 0x10, 0x20, 0x10, 1, 2, 5, 6]);
    assert_eq!(r[3], [7, 7, 7, 7, 7, 7, 7, 7, 0x10, 0x20, 0x10, 0x20, 1, 2, 1, 2]);

    // the next frame keeps what its void blocks cover
    let mut w = BitWriter::new();
    s.types.put(&mut w, code(VOID, 0, 0));
    s.types.put(&mut w, code(SOLID, 0, 9));
    s.types.put(&mut w, code(VOID, 0, 0));
    s.types.put(&mut w, code(VOID, 0, 0));
    dec.decode(None, &w.finish()).unwrap();
    assert_eq!(rows(&dec)[0], [7, 7, 7, 7, 9, 9, 9, 9, 0x20, 0x20, 0x10, 0x10, 3, 4, 1, 2]);

    // no video data repeats the picture
    let before = dec.pixels().to_vec();
    dec.decode(None, &[]).unwrap();
    assert_eq!(dec.pixels(), before);
}

#[test]
fn runs_are_cut_at_the_end_of_the_picture() {
    // run index 63 is 2048 blocks; the picture has 6
    let types = [code(SOLID, 63, 5), code(SOLID, 58, 4), code(SOLID, 59, 3)];
    let s = Stream::new(&[0], &[0], &[0], &types);
    let mut dec = s.decoder(12, 8, b"SMK2");
    for (c, colour) in [(types[0], 5), (types[1], 4), (types[2], 3)] {
        let mut w = BitWriter::new();
        s.types.put(&mut w, c);
        dec.decode(None, &w.finish()).unwrap();
        assert!(dec.pixels().iter().all(|&p| p == colour));
    }
    assert_eq!([run_length(0), run_length(58), run_length(59), run_length(60), run_length(63)], [1, 59, 128, 256, 2048]);
}

#[test]
fn a_size_that_is_not_a_multiple_of_four_leaves_the_edge_alone() {
    let s = Stream::new(&[0], &[0], &[0], &[code(SOLID, 0, 5)]);
    let mut dec = s.decoder(6, 5, b"SMK2");
    // one block; a one-symbol tree takes no bits, so one byte of data is enough
    dec.decode(None, &[0]).unwrap();
    let r = rows(&dec);
    assert_eq!(r[0], [5, 5, 5, 5, 0, 0]);
    assert_eq!(r[4], [0; 6]);
    // smaller than a block: nothing to decode
    let mut tiny = s.decoder(3, 3, b"SMK2");
    tiny.decode(None, &[0]).unwrap();
    assert_eq!(tiny.pixels(), [0; 9]);
}

#[test]
fn smk4_full_block_modes() {
    let types = [code(FULL, 0, 0)];
    let s = Stream::new(&[0], &[0], &[0x0201, 0x0403], &types);
    let block = |bits: &[bool], pairs: &[u16]| {
        let mut dec = s.decoder(4, 4, b"SMK4");
        let mut w = BitWriter::new();
        s.types.put(&mut w, types[0]);
        for &b in bits {
            w.bit(b);
        }
        for &p in pairs {
            s.full.put(&mut w, p);
        }
        dec.decode(None, &w.finish()).unwrap();
        dec.pixels().to_vec()
    };
    // 1: each colour covers 2x2 pixels
    assert_eq!(block(&[true], &[0x0201, 0x0403]), [1, 1, 2, 2, 1, 1, 2, 2, 3, 3, 4, 4, 3, 3, 4, 4]);
    // 0 1: each coded row is doubled down
    assert_eq!(block(&[false, true], &[0x0201, 0x0403, 0x0403, 0x0201]), [3, 4, 1, 2, 3, 4, 1, 2, 1, 2, 3, 4, 1, 2, 3, 4]);
    // 0 0: as in SMK2
    let plain = block(&[false, false], &[0x0201, 0x0403, 0x0201, 0x0403, 0x0201, 0x0403, 0x0201, 0x0403]);
    assert_eq!(plain, [3, 4, 1, 2].repeat(4));
}

#[test]
fn cache_leaves_repeat_recent_values() {
    // solid blocks of colours 1 and 2, and the three cache leaves
    let (a, b) = (code(SOLID, 0, 1), code(SOLID, 0, 2));
    let escapes = [0xE000, 0xE001, 0xE002];
    let types = TreeWriter::new(&[a, b, code(VOID, 4, 0), escapes[0], escapes[1], escapes[2]], escapes);
    let plain = TreeWriter::new(&[0], NO_ESCAPES);
    let s = Stream { maps: plain.clone(), colours: plain.clone(), full: plain, types };
    let mut dec = s.decoder(24, 4, b"SMK2");
    let mut w = BitWriter::new();
    // a, b, then: the most recent (b), the one before (a, which becomes the most recent), the
    // most recent again (a), and the one before (b)
    for c in [a, b, escapes[0], escapes[1], escapes[0], escapes[1]] {
        s.types.put(&mut w, c);
    }
    dec.decode(None, &w.finish()).unwrap();
    let got: Vec<u8> = dec.pixels()[..24].iter().step_by(4).copied().collect();
    assert_eq!(got, [1, 2, 2, 1, 1, 2]);

    // the cache is cleared for each frame: a cache leaf first decodes 0, a mono block
    let mut w = BitWriter::new();
    s.types.put(&mut w, escapes[0]);
    s.types.put(&mut w, code(VOID, 4, 0));
    dec.decode(None, &w.finish()).unwrap();
    let got: Vec<u8> = dec.pixels()[..24].iter().step_by(4).copied().collect();
    assert_eq!(got, [0, 2, 2, 1, 1, 2]);
}

#[test]
fn absent_trees_decode_zero() {
    // four absent trees: every block is a mono block of colour 0
    let params = CodecParams::new(CodecId::SmackerVideo).with_tag(*b"SMK2").with_size(8, 4).with_extradata([0u8; 17]);
    let mut dec = Decoder::new(&params).unwrap();
    dec.pixels.fill(3);
    dec.decode(None, &[0]).unwrap();
    assert_eq!(dec.pixels(), [0; 32]);
}

#[test]
fn palette_chunks_set_keep_and_copy_entries() {
    let s = Stream::new(&[0], &[0], &[0], &[code(VOID, 0, 0)]);
    let mut dec = s.decoder(4, 4, b"SMK2");
    // entry 0 and 1 from six-bit components, then keep the other 254
    dec.decode(Some(&[0x3F, 0x00, 0x10, 0x01, 0x20, 0x3C, 0x80 | 127, 0x80 | 125]), &[]).unwrap();
    assert_eq!(dec.palette()[0], [255, 0, 0x41]);
    assert_eq!(dec.palette()[1], [4, 0x82, 0xF3]);
    assert_eq!(dec.palette()[2], [0, 0, 0]);
    // keep entry 0, copy the old entries 0 and 1 to 1 and 2, set entry 3, keep the rest;
    // bytes after the last entry are padding
    dec.decode(Some(&[0x80, 0x40 | 1, 0, 0x01, 0x01, 0x01, 0x80 | 127, 0x80 | 123, 0, 0]), &[]).unwrap();
    assert_eq!(dec.palette()[..4], [[255, 0, 0x41], [255, 0, 0x41], [4, 0x82, 0xF3], [4, 4, 4]]);
    assert_eq!(dec.to_rgba()[..4], [255, 0, 0x41, 255]);

    let before = *dec.palette();
    // a copy that runs past the last entry, as source and as destination
    assert_eq!(dec.decode(Some(&[0x40 | 9, 250]), &[]), Err(Error::Invalid("palette copy runs past entry 255")));
    assert_eq!(dec.decode(Some(&[0x80 | 127, 0x80 | 120, 0x40 | 9, 0]), &[]), Err(Error::Invalid("palette copy runs past entry 255")));
    assert_eq!(*dec.palette(), before);
}

#[test]
fn a_palette_copy_reads_the_previous_palette() {
    let s = Stream::new(&[0], &[0], &[0], &[code(VOID, 0, 0)]);
    let mut dec = s.decoder(4, 4, b"SMK2");
    dec.decode(Some(&[0x3F, 0x00, 0x00]), &[]).unwrap();
    // entry 0 is set anew, then copied to entry 1: the copy takes the old entry 0
    dec.decode(Some(&[0x00, 0x3F, 0x00, 0x40, 0]), &[]).unwrap();
    assert_eq!(dec.palette()[..2], [[0, 255, 0], [255, 0, 0]]);
}

#[test]
fn a_palette_chunk_that_ends_early_keeps_the_other_entries() {
    let s = Stream::new(&[0], &[0], &[0], &[code(SOLID, 0, 6)]);
    let mut dec = s.decoder(4, 4, b"SMK2");
    dec.decode(Some(&[0x3F, 0x00, 0x00, 0x00, 0x3F, 0x00]), &[]).unwrap();
    // one entry and no more: the rest stay, and the frame's video data is still applied
    dec.decode(Some(&[0x01, 0x01, 0x01]), &[0]).unwrap();
    assert_eq!(dec.palette()[..3], [[4, 4, 4], [0, 255, 0], [0, 0, 0]]);
    assert_eq!(dec.pixels(), [6; 16]);
    // a record cut short by the end of the chunk is dropped: a colour, a copy, nothing at all
    let before = *dec.palette();
    for chunk in [&[0x80, 0x02, 0x03][..], &[0x80, 0x40], &[]] {
        dec.decode(Some(chunk), &[]).unwrap();
        assert_eq!(*dec.palette(), before);
    }
}

#[test]
fn a_long_run_covers_its_whole_length() {
    let types = [code(SOLID, 58, 5), code(VOID, 4, 0)];
    let s = Stream::new(&[0], &[0], &[0], &types);
    // 64 blocks: 59 solid ones, then 5 left alone
    let mut dec = s.decoder(64, 16, b"SMK2");
    let mut w = BitWriter::new();
    s.types.put(&mut w, types[0]);
    s.types.put(&mut w, types[1]);
    dec.decode(None, &w.finish()).unwrap();
    let last_row = rows(&dec)[12];
    assert_eq!(last_row[..44], [5; 44]);
    assert_eq!(last_row[44..], [0; 20]);
}

#[test]
fn the_cache_holds_three_values() {
    let (a, b, c) = (code(SOLID, 0, 1), code(SOLID, 0, 2), code(SOLID, 0, 3));
    let escapes = [0xE000, 0xE001, 0xE002];
    let types = TreeWriter::new(&[a, b, c, code(VOID, 2, 0), escapes[0], escapes[1], escapes[2]], escapes);
    let plain = TreeWriter::new(&[0], NO_ESCAPES);
    let s = Stream { maps: plain.clone(), colours: plain.clone(), full: plain, types };
    let mut dec = s.decoder(32, 4, b"SMK2");
    let mut w = BitWriter::new();
    // a, b, c, then twice the third most recent value: a (the cache is then a, c, b), and b
    for sym in [a, b, c, escapes[2], escapes[2], code(VOID, 2, 0)] {
        s.types.put(&mut w, sym);
    }
    dec.decode(None, &w.finish()).unwrap();
    let got: Vec<u8> = dec.pixels()[..32].iter().step_by(4).copied().collect();
    assert_eq!(got, [1, 2, 3, 1, 2, 0, 0, 0]);
}

#[test]
fn a_leaf_is_a_cache_leaf_for_the_first_escape_that_names_it_only() {
    // the second escape is 0. The leaf of the first escape holds 0 once it is a cache leaf,
    // and must not take the second slot from the leaf whose value is 0.
    let (a, b) = (code(SOLID, 0, 1), code(SOLID, 0, 2));
    let escapes = [0xE000, 0x0000, 0xE002];
    let types = TreeWriter::new(&[0x0000, escapes[0], a, b, code(VOID, 0, 0)], escapes);
    let plain = TreeWriter::new(&[0], NO_ESCAPES);
    let s = Stream { maps: plain.clone(), colours: plain.clone(), full: plain, types };
    let mut dec = s.decoder(16, 4, b"SMK2");
    let mut w = BitWriter::new();
    // a, b, the second most recent value (a), and a void block
    for sym in [a, b, 0x0000, code(VOID, 0, 0)] {
        s.types.put(&mut w, sym);
    }
    dec.decode(None, &w.finish()).unwrap();
    let got: Vec<u8> = dec.pixels()[..16].iter().step_by(4).copied().collect();
    assert_eq!(got, [1, 2, 1, 0]);
}

#[test]
fn a_tree_is_bounded_whatever_size_the_header_declares() {
    // a present tree with no byte trees and zero escapes, then branches without end
    let mut w = BitWriter::new();
    w.bit(true);
    w.bits(0, 2);
    for _ in 0..3 {
        w.bits(0, 16);
    }
    for _ in 0..(MAX_TREE_NODES + 1000) {
        w.bit(true);
    }
    let mut extradata = [u32::MAX.to_le_bytes(); 4].concat();
    extradata.extend(w.finish());
    let params = CodecParams::new(CodecId::SmackerVideo).with_tag(*b"SMK2").with_size(8, 8).with_extradata(extradata);
    assert_eq!(Decoder::new(&params).err(), Some(Error::Invalid("tree has too many nodes")));
}

#[test]
fn a_tree_of_every_sixteen_bit_value_is_read_and_a_larger_one_is_not() {
    let plain = TreeWriter::new(&[0], NO_ESCAPES);
    let with_types = |symbols: &[u16]| {
        let types = TreeWriter::new(symbols, NO_ESCAPES);
        let s = Stream { maps: plain.clone(), colours: plain.clone(), full: plain.clone(), types };
        Decoder::new(&s.params(8, 8, b"SMK2")).map(|_| ())
    };
    // 65536 leaves: 131071 nodes
    let mut symbols: Vec<u16> = (0..=u16::MAX).collect();
    assert_eq!(with_types(&symbols), Ok(()));
    // one leaf more, in a whole tree that declares its true size
    symbols.push(0);
    assert_eq!(with_types(&symbols), Err(Error::Invalid("tree has too many nodes")));
}

#[test]
fn the_picture_size_is_limited() {
    let s = Stream::new(&[0], &[0], &[0], &[code(VOID, 0, 0)]);
    assert!(Decoder::new(&s.params(4096, 4096, b"SMK2")).is_ok());
    assert_eq!(Decoder::new(&s.params(4097, 4096, b"SMK2")).err(), Some(Error::TooLarge(4097, 4096)));
    assert_eq!(Decoder::new(&s.params(1 << 20, 17, b"SMK2")).err(), Some(Error::TooLarge(1 << 20, 17)));
}

#[test]
fn every_six_bit_level_expands_to_eight_bits() {
    assert_eq!([expand6(0), expand6(1), expand6(16), expand6(32), expand6(63)], [0, 4, 0x41, 0x82, 255]);
    assert_eq!(expand6(0xC1), 4);
}

#[test]
fn bad_parameters_and_trees_are_errors() {
    let s = Stream::new(&[1, 2, 3], &[4], &[5], &[code(SOLID, 0, 1), code(MONO, 0, 0)]);
    let good = s.params(8, 8, b"SMK2");
    assert!(Decoder::new(&good).is_ok());

    let mut p = good.clone();
    p.codec = CodecId::Vp8;
    assert_eq!(Decoder::new(&p).err(), Some(Error::Invalid("not Smacker video")));
    let mut p = good.clone();
    p.tag = Some(*b"BIKi");
    assert_eq!(Decoder::new(&p).err(), Some(Error::Invalid("unknown version")));
    let mut p = good.clone();
    p.width = None;
    assert_eq!(Decoder::new(&p).err(), Some(Error::Invalid("no picture size")));
    let p = good.clone().with_size(u32::MAX, u32::MAX);
    assert_eq!(Decoder::new(&p).err(), Some(Error::TooLarge(u32::MAX, u32::MAX)));

    // tree data cut short at every length
    for n in 0..good.extradata.len() {
        let p = good.clone().with_extradata(&good.extradata[..n]);
        assert_eq!(Decoder::new(&p).err(), Some(Error::Truncated), "{n} bytes");
    }
    // a tree may have four nodes more than its declared size gives (four bytes a node), and no
    // more: the first tree has five nodes
    let mut p = good.clone();
    p.extradata[..4].copy_from_slice(&4u32.to_le_bytes());
    assert!(Decoder::new(&p).is_ok());
    p.extradata[..4].fill(0);
    assert_eq!(Decoder::new(&p).err(), Some(Error::Invalid("tree has too many nodes")));
    // a hostile size is bounded by the data: branches without end
    let mut data = vec![0xFF; 16];
    data.extend([0xFF; 64]);
    let p = good.with_extradata(data);
    assert!(Decoder::new(&p).is_err());
}

#[test]
fn a_picture_cut_short_is_an_error_and_keeps_the_blocks_before_it() {
    let types = [code(SOLID, 0, 1), code(MONO, 0, 0)];
    let s = Stream::new(&[1, 2], &[4, 5], &[5], &types);
    // sixteen blocks, and one byte of data: eight one-bit solid codes
    let mut dec = s.decoder(64, 4, b"SMK2");
    assert_eq!(dec.decode(None, &[0]), Err(Error::Truncated));
    assert_eq!(rows(&dec)[0], [[1u8; 32], [0; 32]].concat());
}

#[test]
fn hostile_input_never_panics() {
    let types = [code(SOLID, 1, 7), code(VOID, 0, 0), code(MONO, 2, 0), code(FULL, 0, 0), 0xE000, 0xE001];
    let t = |s: &[u16]| TreeWriter::new(s, [0xE000, 0xE001, 0xE002]);
    let s = Stream { maps: t(&[0xA5C3, 0x0001, 0xE002]), colours: t(&[0x2010, 0x0102]), full: t(&[0x0201, 0x0403, 0xE000]), types: t(&types) };
    let mut w = BitWriter::new();
    for i in 0..400u32 {
        w.bits(i.wrapping_mul(0x9E37_79B9) >> 7, 13);
    }
    let video = w.finish();
    let palette = [0x3F, 0x00, 0x10, 0x40 | 5, 3, 0x80 | 20, 0x01, 0x02, 0x03, 0x80 | 127, 0x80 | 100];
    for tag in [b"SMK2", b"SMK4"] {
        let params = s.params(40, 24, tag);
        // seed: the tree data, then the palette chunk, then the video data
        let mut seed = params.extradata.clone();
        let split = seed.len();
        seed.extend(palette);
        seed.extend(&video);
        craft_media_testkit::hostile::hammer("smacker video", &seed, 20_000, move |d| {
            let (trees, rest) = d.split_at(split.min(d.len()));
            let (pal, video) = rest.split_at(palette.len().min(rest.len()));
            if let Ok(mut dec) = Decoder::new(&params.clone().with_extradata(trees)) {
                let _ = dec.decode(Some(pal), video);
                let _ = dec.decode(None, video);
                let _ = dec.to_rgba();
            }
        });
    }
}

/// The picture converts into a `VideoFrame` of R, G, B planes holding the colours the decoder
/// gives as RGBA.
#[test]
fn the_picture_converts_to_an_rgb_video_frame() {
    use crate::frame::{ColorFamily, Samples, VideoFrame};
    let s = Stream::new(&[0], &[0], &[0], &[code(SOLID, 0, 7), code(SOLID, 0, 9)]);
    let mut dec = s.decoder(8, 4, b"SMK2");
    let mut w = BitWriter::new();
    s.types.put(&mut w, code(SOLID, 0, 7));
    s.types.put(&mut w, code(SOLID, 0, 9));
    // keep entries 0 to 6, set 7, keep 8, set 9
    dec.decode(Some(&[0x80 | 6, 0x3F, 0x00, 0x10, 0x80, 0x01, 0x20, 0x3C]), &w.finish()).unwrap();
    assert_eq!((dec.palette()[7], dec.palette()[9]), ([255, 0, 0x41], [4, 0x82, 0xF3]));
    assert_eq!(rows(&dec)[0], [7, 7, 7, 7, 9, 9, 9, 9]);

    let f = VideoFrame::try_from(&dec).unwrap();
    assert_eq!((f.width(), f.height(), f.family(), f.bit_depth()), (8, 4, ColorFamily::Rgb, 8));
    assert_eq!(f.color.matrix, 0);
    let plane = |p: usize| match f.plane(p).map(|p| p.samples()) {
        Some(Samples::U8(s)) => s.clone(),
        other => panic!("plane {p}: {other:?}"),
    };
    let (r, g, b) = (plane(0), plane(1), plane(2));
    let rgba: Vec<u8> = (0..32).flat_map(|i| [r[i], g[i], b[i], 255]).collect();
    assert_eq!(rgba, dec.to_rgba());
    assert_eq!(r[..8], [255, 255, 255, 255, 4, 4, 4, 4]);
}
