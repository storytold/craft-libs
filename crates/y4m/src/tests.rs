//! Ported from FilmCraft's Y4M tests (`crates/codecs/src/y4m.rs`, `tests/it/y4m_file.rs`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

/// A Y4M file: `header` after the signature, `frames` as raw plane bytes, each after `frame_line`.
fn y4m(header: &str, frame_line: &str, frames: &[Vec<u8>]) -> Vec<u8> {
    let mut out = format!("YUV4MPEG2 {header}\n").into_bytes();
    for f in frames {
        out.extend_from_slice(frame_line.as_bytes());
        out.push(b'\n');
        out.extend_from_slice(f);
    }
    out
}

/// One 8-bit frame: every Y sample `y`, every Cb `cb`, every Cr `cr`.
fn planes8(luma: usize, chroma: usize, y: u8, cb: u8, cr: u8) -> Vec<u8> {
    [vec![y; luma], vec![cb; chroma], vec![cr; chroma]].concat()
}

fn open(bytes: &[u8]) -> Result<Stream> {
    Stream::from_bytes(bytes, &Limits::default())
}

fn frame<'a>(s: &Stream, bytes: &'a [u8], index: usize) -> &'a [u8] {
    let r = s.frame(index).unwrap();
    &bytes[r.start as usize..r.end as usize]
}

#[test]
fn colour_space_names() {
    let l = |c: Option<Chroma>, bits, alpha| Some(Layout { chroma: c, bits, alpha });
    assert_eq!(Layout::parse("420jpeg"), l(Some(Chroma::C420), 8, false));
    assert_eq!(Layout::parse("420p10"), l(Some(Chroma::C420), 10, false));
    assert_eq!(Layout::parse("422p12"), l(Some(Chroma::C422), 12, false));
    assert_eq!(Layout::parse("444alpha"), l(Some(Chroma::C444), 8, true));
    assert_eq!(Layout::parse("mono"), l(None, 8, false));
    assert_eq!(Layout::parse("mono16"), l(None, 16, false));
    for bad in ["411", "420p", "420p7", "420p17", "mono10p12", "444alphap10", "", "p10"] {
        assert_eq!(Layout::parse(bad), None, "{bad}");
    }
}

#[test]
fn a_420_file_is_indexed_frame_by_frame() {
    // 6 × 4, 4:2:0 (chroma 3 × 2), three frames at 25 fps.
    let frames: Vec<Vec<u8>> = (0..3).map(|k| planes8(24, 6, 16 + 50 * k, 100 + k, 200 - k)).collect();
    let bytes = y4m("W6 H4 F25:1 Ip A1:1 C420jpeg", "FRAME", &frames);
    assert!(sniff(&bytes));
    let s = open(&bytes).unwrap();
    let h = &s.header;
    assert_eq!((h.width, h.height, h.frame_rate, h.pixel_aspect, h.full_range), (6, 4, Some((25, 1)), Some((1, 1)), false));
    assert_eq!(h.layout, Layout { chroma: Some(Chroma::C420), bits: 8, alpha: false });
    assert_eq!((h.plane_samples(), s.frame_bytes, s.frames.len()), ((24, 6, 0), 36, 3));
    let p = h.planes();
    assert_eq!((p.y.clone(), p.cb.clone(), p.cr.clone(), p.alpha.clone()), (0..24, 24..30, 30..36, 36..36));
    for k in 0..3u8 {
        let f = frame(&s, &bytes, usize::from(k));
        assert!(
            f[p.y.clone()].iter().all(|&v| v == 16 + 50 * k)
                && f[p.cb.clone()].iter().all(|&v| v == 100 + k)
                && f[p.cr.clone()].iter().all(|&v| v == 200 - k)
        );
    }
    assert_eq!(s.frame(3), None);
}

#[test]
fn high_bit_depth_frame_parameters_aspect_and_range() {
    // 4 × 2, 4:2:2 10-bit (little-endian), 30000:1001, 4:3 pixels, full range, and frame lines
    // carrying their own parameters.
    let mut data = Vec::new();
    for (n, v) in [(8, 940u16), (4, 512), (4, 64)] {
        for _ in 0..n {
            data.extend(v.to_le_bytes());
        }
    }
    let bytes = y4m("W4 H2 F30000:1001 It A4:3 C422p10 XCOLORRANGE=FULL", "FRAME Ip XFOO=1", &[data.clone(), data]);
    let s = open(&bytes).unwrap();
    let h = &s.header;
    assert_eq!((h.frame_rate, h.pixel_aspect, h.full_range), (Some((30000, 1001)), Some((4, 3)), true));
    assert_eq!((h.layout, h.layout.bytes_per_sample(), s.frames.len()), (Layout { chroma: Some(Chroma::C422), bits: 10, alpha: false }, 2, 2));
    let (p, f) = (h.planes(), frame(&s, &bytes, 1));
    assert_eq!(samples_u16(&f[p.y], 10), [940; 8]);
    assert_eq!(samples_u16(&f[p.cb], 10), [512; 4]);
    assert_eq!(samples_u16(&f[p.cr], 10), [64; 4]);
    // Samples above the depth's largest value are clamped.
    assert_eq!(samples_u16(&[0xFF, 0xFF, 0x00, 0x04, 0x01], 10), [1023, 1023]);
}

#[test]
fn mono_alpha_and_defaults() {
    let bytes = y4m("W3 H3 Cmono", "FRAME", &[vec![77; 9]]);
    let s = open(&bytes).unwrap();
    assert_eq!((s.header.layout.chroma, s.header.frame_rate, s.header.pixel_aspect), (None, None, None));
    assert_eq!((s.header.plane_samples(), s.header.chroma_samples(Chroma::C420), s.header.chroma_samples(Chroma::C444)), ((9, 0, 0), 4, 9));
    let p = s.header.planes();
    assert!(p.cb.is_empty() && p.cr.is_empty() && p.alpha.is_empty());
    // No colour space means 4:2:0; an unknown aspect (0:0) is none.
    let s = open(&y4m("W2 H2 A0:0", "FRAME", &[vec![0; 6]])).unwrap();
    assert_eq!((s.header.layout.chroma, s.header.pixel_aspect), (Some(Chroma::C420), None));
    let bytes = y4m("W2 H1 C444alpha", "FRAME", &[vec![1, 1, 2, 2, 3, 3, 4, 4]]);
    let s = open(&bytes).unwrap();
    let p = s.header.planes();
    assert_eq!((s.header.plane_samples(), p.alpha.clone()), ((2, 2, 2), 6..8));
    assert_eq!(&frame(&s, &bytes, 0)[p.alpha], &[4, 4]);
}

#[test]
fn a_file_opens_through_a_reader_in_small_reads() {
    let bytes = y4m("W2 H2 F25:1 C444", "FRAME", &[planes8(4, 4, 50, 60, 70), planes8(4, 4, 90, 60, 70)]);
    let mut longest = 0;
    let s = Stream::open(
        bytes.len() as u64,
        |pos, n| {
            longest = longest.max(n);
            bytes.get(pos as usize..pos as usize + n).map(<[u8]>::to_vec)
        },
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(frame(&s, &bytes, 1)[0], 90);
    // Only lines are read, never a frame's planes.
    assert!(longest <= 128, "{longest}");
    // A read that fails ends the index, or the open when it is the header's.
    assert_eq!(Stream::open(bytes.len() as u64, |_, _| None, &Limits::default()), Err(Error::Malformed("no stream header")));
    let first = s.frames[0];
    let s = Stream::open(
        bytes.len() as u64,
        |pos, n| if pos > first { None } else { bytes.get(pos as usize..pos as usize + n).map(<[u8]>::to_vec) },
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(s.frames.len(), 1);
}

#[test]
fn a_cut_or_damaged_frame_ends_the_file() {
    let frames = [planes8(4, 1, 1, 2, 3), planes8(4, 1, 4, 5, 6)];
    let good = y4m("W2 H2", "FRAME", &frames);
    assert_eq!(open(&good).unwrap().frames.len(), 2);
    assert_eq!(open(&good[..good.len() - 1]).unwrap().frames.len(), 1);
    let mut junk = good.clone();
    let second = open(&good).unwrap().frames[1] as usize - 6;
    junk[second] = b'X';
    assert_eq!(open(&junk).unwrap().frames.len(), 1);
    assert_eq!(open(&y4m("W2 H2", "FRAME", &[vec![0; 5]])), Err(Error::Malformed("no complete frames")));
    assert_eq!(open(b"YUV4MPEG2 W2 H2"), Err(Error::Malformed("no stream header")));
}

#[test]
fn bad_headers_are_errors() {
    let h = |header: &str| open(&y4m(header, "FRAME", &[vec![0; 36]]));
    assert_eq!(h("W0 H4 F25:1"), Err(Error::Malformed("missing or zero size")));
    assert_eq!(h("H4 F25:1"), Err(Error::Malformed("missing or zero size")));
    assert_eq!(h("Wx H4"), Err(Error::Malformed("invalid width")));
    assert_eq!(h("W6 H-4"), Err(Error::Malformed("invalid height")));
    for rate in ["F0:0", "F25:0", "F-5:1", "F25", "Fa:b"] {
        assert_eq!(h(&format!("W6 H4 {rate}")), Err(Error::Malformed("invalid frame rate")), "{rate}");
    }
    assert_eq!(h("W6 H4 F25:1 C411"), Err(Error::UnsupportedColourSpace("411".into())));
    assert_eq!(h("W6 H4 F25:1 Cbogus"), Err(Error::UnsupportedColourSpace("bogus".into())));
    assert!(h("W6 H4 F1:9223372036854775807").is_ok());
    assert_eq!(Header::parse(b"YUV4MPEG W2 H2", &Limits::default()), Err(Error::Malformed("not a YUV4MPEG2 file")));
    assert!(!sniff(b"YUV4MPEG2X") && !sniff(b"YUV4MPEG") && sniff(b"YUV4MPEG2\n"));
}

#[test]
fn limits() {
    let h = |header: &str| open(&y4m(header, "FRAME", &[vec![0; 36]]));
    assert_eq!(h("W99999999 H99999999 F25:1"), Err(Error::LimitExceeded("pixel count")));
    assert_eq!(h("W4294967295 H4294967295"), Err(Error::LimitExceeded("pixel count")));
    let bytes = y4m("W2 H2", "FRAME", &[vec![0; 6], vec![0; 6], vec![0; 6]]);
    assert!(Stream::from_bytes(&bytes, &Limits { max_pixels: 4, ..Limits::default() }).is_ok());
    assert_eq!(Stream::from_bytes(&bytes, &Limits { max_pixels: 3, ..Limits::default() }), Err(Error::LimitExceeded("pixel count")));
    // The index stops at the frame limit.
    assert_eq!(Stream::from_bytes(&bytes, &Limits { max_frames: 2, ..Limits::default() }).unwrap().frames.len(), 2);
    // A header or frame line longer than the line limit is not read.
    let long = y4m(&format!("W2 H2 X{}", "y".repeat(300)), "FRAME", &[vec![0; 6]]);
    assert!(open(&long).is_ok());
    assert_eq!(Stream::from_bytes(&long, &Limits { max_line: 256, ..Limits::default() }), Err(Error::Malformed("no stream header")));
    let long_frame = y4m("W2 H2", &format!("FRAME X{}", "y".repeat(300)), &[vec![0; 6]]);
    assert!(open(&long_frame).is_ok());
    assert_eq!(Stream::from_bytes(&long_frame, &Limits { max_line: 256, ..Limits::default() }), Err(Error::Malformed("no complete frames")));
    assert_eq!(Limits::default(), Limits { max_pixels: 1 << 28, max_line: 64 << 10, max_frames: 1 << 24 });
}

#[test]
fn damaged_and_hostile_files_never_panic() {
    let good = y4m("W6 H4 F25:1 C420jpeg", "FRAME", &[planes8(24, 6, 1, 2, 3), planes8(24, 6, 4, 5, 6)]);
    let mut cases: Vec<Vec<u8>> = (0..good.len()).map(|n| good[..n].to_vec()).collect();
    for i in 0..good.len() {
        let mut b = good.clone();
        b[i] ^= 0x41;
        cases.push(b);
    }
    for b in cases {
        if let Ok(s) = open(&b) {
            for k in 0..s.frames.len() {
                let r = s.frame(k).unwrap();
                assert!(r.end <= b.len() as u64);
            }
        }
    }
}

/// A header parameter that starts with a multi-byte character is skipped like any unknown tag
/// (splitting it at byte 1 would cut the character in two).
#[test]
fn multibyte_header_param_is_ignored() {
    let mut data = "YUV4MPEG2 W2 H2 éX\nFRAME\n".as_bytes().to_vec();
    data.extend([0u8; 6]);
    let s = open(&data).unwrap();
    assert_eq!((s.header.width, s.header.height, s.frames.len()), (2, 2, 1));
    // Bytes that are not UTF-8 at all are skipped the same way.
    let mut data = b"YUV4MPEG2 W2 H2 \xFF\xFEX\nFRAME\n".to_vec();
    data.extend([0u8; 6]);
    assert!(open(&data).is_ok());
}
