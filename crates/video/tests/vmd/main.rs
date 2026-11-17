//! VMD video decoder tests. ffmpeg reads VMD but cannot write it, so the files come from the
//! test kit's VMD writer, which packs random pictures with every coding mode: the three
//! rendering methods, LZ packing with short and long chains, partial updates and palette
//! changes. The decoder must return the pictures that were packed, and the same pictures as
//! ffmpeg (skipped without ffmpeg).

use std::io::Read;
use std::process::{Command, Stdio};

use craft_media::vmd::{FrameKind, open};
use craft_media_testkit::hostile::Rng;
use craft_media_testkit::vmd::{Frame, Spec, build, delta_lines, lz_pack, palette_chunk};
use craft_video::vmd::{Decoder, Error, Rect};

/// A file and the picture (indices and 6-bit palette) expected after each block.
struct Movie {
    bytes: Vec<u8>,
    width: usize,
    pictures: Vec<(Vec<u8>, [u8; 768])>,
    /// How each frame was packed, for failure messages.
    notes: Vec<String>,
}

/// Change a random rectangle of `pic` with stretches of noise, flat colour, repeated pairs,
/// 0xFF and untouched pixels; returns the rectangle.
fn mutate(rng: &mut Rng, pic: &mut [u8], w: usize, h: usize) -> [u16; 4] {
    let x0 = rng.below(w);
    let x1 = x0 + rng.below(w - x0);
    let y0 = rng.below(h);
    let y1 = y0 + rng.below(h - y0);
    for y in y0..=y1 {
        let mut style = rng.below(5);
        let (a, b) = (rng.next_u64() as u8, rng.next_u64() as u8);
        for x in x0..=x1 {
            if rng.below(12) == 0 {
                style = rng.below(5);
            }
            let p = &mut pic[y * w + x];
            match style {
                0 => *p = rng.next_u64() as u8,
                1 => *p = a,
                2 => *p = if x % 2 == 0 { a } else { b },
                3 => *p = 0xFF,
                _ => {}
            }
        }
    }
    [x0 as u16, y0 as u16, x1 as u16, y1 as u16]
}

fn movie(seed: u64, w: usize, h: usize, frames: usize) -> Movie {
    let mut rng = Rng(seed | 1);
    let mut spec = Spec::new(w as u16, h as u16);
    for p in spec.palette.iter_mut() {
        *p = rng.below(64) as u8;
    }
    let mut palette = spec.palette;
    let mut prev = vec![0u8; w * h];
    let mut blocks = Vec::new();
    let mut pictures = Vec::new();
    let mut notes = Vec::new();
    for n in 0..frames {
        let mut cur = prev.clone();
        let rect = if n == 0 {
            for p in cur.iter_mut() {
                *p = rng.next_u64() as u8;
            }
            mutate(&mut rng, &mut cur, w, h);
            [0, 0, w as u16 - 1, h as u16 - 1]
        } else {
            mutate(&mut rng, &mut cur, w, h)
        };
        let method = 1 + rng.below(3) as u8;
        // the writer's method 3 needs two pixels a line; the first frame has no picture before
        // it to keep pixels from, and ffmpeg refuses methods 1 and 3 there
        let method = match method {
            _ if n == 0 => 2,
            3 if rect[0] == rect[2] => 1,
            m => m,
        };
        let mut body = match method {
            2 => (rect[1]..=rect[3]).flat_map(|y| cur[usize::from(y) * w..][usize::from(rect[0])..=usize::from(rect[2])].to_vec()).collect(),
            _ => delta_lines(&prev, &cur, w, rect, method == 3, &mut rng),
        };
        // ffmpeg refuses LZ packing of a body of a byte or two, which no encoder would pack
        let packing = if body.len() < 16 { 0 } else { rng.below(3) };
        if packing > 0 {
            body = lz_pack(&body, packing == 2, &mut rng);
        }
        let new_palette = rng.below(4) == 0;
        let mut data = Vec::new();
        if new_palette {
            for p in palette.iter_mut() {
                *p = rng.below(64) as u8;
            }
            data = palette_chunk(&palette);
        }
        data.push(method | if packing > 0 { 0x80 } else { 0 });
        data.extend_from_slice(&body);
        notes.push(format!("method {method}, packing {packing}, new palette {new_palette}, rect {rect:?}, body {body:02x?}"));
        blocks.push(vec![Frame::video(rect, new_palette, data)]);
        pictures.push((cur.clone(), palette));
        prev = cur;
    }
    Movie { bytes: build(&spec, &blocks), width: w, pictures, notes }
}

/// The pictures of a file as the decoder gives them: indices and RGB, one for each block.
fn decode_all(bytes: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let file = open(bytes).unwrap();
    let mut decoder = Decoder::new(file.header_bytes()).unwrap();
    let mut out = Vec::new();
    for (i, frame) in file.frames.iter().enumerate() {
        let FrameKind::Video { new_palette, .. } = frame.kind else { continue };
        let data = file.read_frame(bytes, i).unwrap();
        match file.picture_rect(i) {
            Some(rect) => decoder.decode(rect.into(), new_palette, &data).unwrap(),
            None if new_palette => decoder.decode_palette(&data).unwrap(),
            None => {}
        }
        out.push((decoder.indices().to_vec(), decoder.rgb24()));
    }
    out
}

fn rgb(indices: &[u8], palette: &[u8; 768]) -> Vec<u8> {
    indices.iter().flat_map(|&i| palette[usize::from(i) * 3..][..3].iter().map(|&v| v << 2 | v >> 4)).collect()
}

/// (seed, width, height, frames): small pictures, one wider than the longest run (128) and one
/// larger than the LZ window (4096).
const MOVIES: [(u64, usize, usize, usize); 5] = [(1, 40, 30, 40), (2, 300, 5, 30), (3, 1, 1, 6), (4, 7, 64, 30), (5, 96, 80, 12)];

/// Each picture converts into a `VideoFrame` of R, G, B planes holding the colours the decoder
/// gives as RGB.
#[test]
fn pictures_convert_to_rgb_video_frames() {
    use craft_video::frame::{ColorFamily, Samples, VideoFrame};
    for (seed, w, h, frames) in MOVIES {
        let m = movie(seed, w, h, frames);
        let file = open(&m.bytes).unwrap();
        let mut decoder = Decoder::new(file.header_bytes()).unwrap();
        for (i, frame) in file.frames.iter().enumerate() {
            let FrameKind::Video { new_palette, .. } = frame.kind else { continue };
            let data = file.read_frame(&m.bytes, i).unwrap();
            match file.picture_rect(i) {
                Some(rect) => decoder.decode(rect.into(), new_palette, &data).unwrap(),
                None if new_palette => decoder.decode_palette(&data).unwrap(),
                None => {}
            }
            let f = VideoFrame::try_from(&decoder).unwrap();
            assert_eq!((f.width(), f.height(), f.family(), f.bit_depth()), (w as u32, h as u32, ColorFamily::Rgb, 8));
            assert_eq!(f.color.matrix, 0);
            let plane = |p: usize| match f.plane(p).map(|p| p.samples()) {
                Some(Samples::U8(s)) => s.clone(),
                other => panic!("plane {p}: {other:?}"),
            };
            let (r, g, b) = (plane(0), plane(1), plane(2));
            let interleaved: Vec<u8> = (0..w * h).flat_map(|i| [r[i], g[i], b[i]]).collect();
            assert!(interleaved == decoder.rgb24(), "movie {seed}: frame {i}");
        }
    }
}

#[test]
fn decodes_the_pictures_that_were_packed() {
    for (seed, w, h, frames) in MOVIES {
        let m = movie(seed, w, h, frames);
        let got = decode_all(&m.bytes);
        assert_eq!(got.len(), frames);
        for (n, ((indices, rgb24), (want, palette))) in got.iter().zip(&m.pictures).enumerate() {
            assert!(indices == want, "movie {seed}: indices of frame {n}");
            assert!(*rgb24 == rgb(want, palette), "movie {seed}: colours of frame {n}");
        }
    }
}

#[test]
fn matches_ffmpeg() {
    let Some(ffmpeg) = craft_media_testkit::ffmpeg_or_skip("vmd video oracle") else { return };
    let dir = craft_media_testkit::private_dir("vmd-video");
    for (seed, w, h, frames) in MOVIES {
        let m = movie(seed, w, h, frames);
        let path = dir.join(format!("movie{seed}.vmd"));
        let run = |bytes: &[u8]| {
            std::fs::write(&path, bytes).unwrap();
            Command::new(&ffmpeg)
                .args(["-v", "error", "-i"])
                .arg(&path)
                .args(["-an", "-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
                .output()
                .unwrap()
        };
        let out = run(&m.bytes);
        if !out.status.success() || !out.stderr.is_empty() {
            // the same movie cut after each frame in turn shows which frame ffmpeg refuses
            let bad = (1..=frames).find(|&k| !run(&movie(seed, w, h, k).bytes).stderr.is_empty()).map_or(0, |k| k - 1);
            panic!("movie {seed}: ffmpeg refuses frame {bad} ({}): {}", m.notes[bad], String::from_utf8_lossy(&out.stderr));
        }
        let theirs: Vec<&[u8]> = out.stdout.chunks(m.width * h * 3).collect();
        let ours = decode_all(&m.bytes);
        assert_eq!(theirs.len(), ours.len(), "movie {seed}: frame count");
        for (n, (theirs, (_, ours))) in theirs.iter().zip(&ours).enumerate() {
            assert!(*theirs == ours.as_slice(), "movie {seed}: frame {n} differs from ffmpeg");
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A 16x2 decoder whose picture is all 0x40.
fn flat() -> Decoder {
    let header = build(&Spec::new(16, 2), &[]);
    let mut d = Decoder::new(&header).unwrap();
    let mut key = vec![2u8];
    key.extend([0x40; 32]);
    d.decode(FULL, false, &key).unwrap();
    d
}

const FULL: Rect = Rect { left: 0, top: 0, right: 15, bottom: 1 };

/// Method 3 lines as ffmpeg was seen to decode them: the second line is skipped whole.
#[test]
fn run_length_lines() {
    let cases: [(&[u8], [u8; 16]); 5] = [
        // 16 bytes that do not begin with 0xFF are copied
        (&[0x8F, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16], [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]),
        // 0xFF: a pair three times, two pairs as they are, a pair three times
        (
            &[0x8F, 0xFF, 3, 0xA0, 0xB0, 0x82, 1, 2, 3, 4, 3, 0xC0, 0xD0],
            [0xA0, 0xB0, 0xA0, 0xB0, 0xA0, 0xB0, 1, 2, 3, 4, 0xC0, 0xD0, 0xC0, 0xD0, 0xC0, 0xD0],
        ),
        // an odd run starts with a single byte; the last pixel is a plain run of one
        (
            &[0x8E, 0xFF, 0xE0, 7, 0xA0, 0xB0, 0x80, 0xE4],
            [0xE0, 0xA0, 0xB0, 0xA0, 0xB0, 0xA0, 0xB0, 0xA0, 0xB0, 0xA0, 0xB0, 0xA0, 0xB0, 0xA0, 0xB0, 0xE4],
        ),
        // skip 8, then run-length
        (&[7, 0x87, 0xFF, 4, 0x50, 0x60], [0x40, 0x40, 0x40, 0x40, 0x40, 0x40, 0x40, 0x40, 0x50, 0x60, 0x50, 0x60, 0x50, 0x60, 0x50, 0x60]),
        // skip 8, then plain
        (&[7, 0x87, 1, 2, 3, 4, 5, 6, 7, 8], [0x40, 0x40, 0x40, 0x40, 0x40, 0x40, 0x40, 0x40, 1, 2, 3, 4, 5, 6, 7, 8]),
    ];
    for (line, want) in cases {
        let mut d = flat();
        let mut data = vec![3u8];
        data.extend_from_slice(line);
        data.push(15);
        d.decode(FULL, false, &data).unwrap();
        assert_eq!(d.indices()[..16], want, "{line:x?}");
        assert_eq!(d.indices()[16..], [0x40; 16], "{line:x?}");
    }
    // method 1 has no run-length coding: 0xFF is a pixel
    let mut d = flat();
    d.decode(FULL, false, &[1, 0x81, 0xFF, 9, 13, 15]).unwrap();
    assert_eq!(d.indices()[..3], [0xFF, 9, 0x40]);
}

#[test]
fn lz_chains() {
    let unpack = |packed: &[u8]| {
        let mut d = flat();
        let mut data = vec![0x82u8];
        data.extend_from_slice(packed);
        d.decode(FULL, false, &data).map(|()| d.indices().to_vec())
    };
    // three literals, a chain that repeats them, then chains into the window's fill of 0x20:
    // 5 bytes, 18 bytes (the longest short chain), 3 bytes
    let short = [32, 0, 0, 0, 0b0000_0111, 0x90, 0xA0, 0xB0, 0xEE, 0xF0, 0x00, 0x02, 0x00, 0x0F, 0x00, 0x00];
    let mut want = vec![0x90, 0xA0, 0xB0, 0x90, 0xA0, 0xB0];
    want.resize(32, 0x20);
    assert_eq!(unpack(&short).unwrap(), want);
    // with the marker the window starts at 0x111, and a length of 18 takes one more byte: 18 + 5
    let long = [32, 0, 0, 0, 0x34, 0x12, 0x78, 0x56, 0b0000_0111, 0x90, 0xA0, 0xB0, 0x11, 0x10, 0x00, 0x0F, 5];
    // 29 of the 32 bytes arrive (3 literals, 3 and 23 chained): the first row is painted, the
    // second, three bytes short, is not
    assert_eq!(unpack(&long), Err(Error::Invalid("the frame data ends early")));
    let mut d = flat();
    let mut data = vec![0x82u8];
    data.extend_from_slice(&long);
    let _ = d.decode(FULL, false, &data);
    let mut want = vec![0x90, 0xA0, 0xB0, 0x90, 0xA0, 0xB0];
    want.resize(16, 0x20);
    want.extend([0x40; 16]);
    assert_eq!(d.indices(), want);
    // a chain that overlaps what it writes repeats it: one literal, then 18 copies of it
    let overlap = [32, 0, 0, 0, 0b0000_0001, 7, 0xEE, 0xFF, 0xEE, 0xFA];
    assert_eq!(unpack(&overlap).unwrap(), [7; 32]);
    // a tag of 0xFF with more than 8 bytes left is 8 raw bytes
    let mut raw = vec![32, 0, 0, 0];
    for chunk in (0u8..32).collect::<Vec<_>>().chunks(8) {
        raw.push(0xFF);
        raw.extend_from_slice(chunk);
    }
    assert_eq!(unpack(&raw).unwrap(), (0u8..32).collect::<Vec<_>>());
    // with 8 bytes or fewer left, a tag of 0xFF is eight literal bits, of which only the bytes
    // left are read: 13 bytes are 8 raw and 5 literals
    let mut d = flat();
    let tail = [0x82, 13, 0, 0, 0, 0xFF, 1, 2, 3, 4, 5, 6, 7, 8, 0xFF, 9, 10, 11, 12, 13];
    d.decode(Rect { right: 12, bottom: 0, ..FULL }, false, &tail).unwrap();
    assert_eq!(d.indices()[..14], [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 0x40]);
}

/// A frame that says it unpacks to the whole unpack buffer costs no more than its picture: the
/// same 8 packed bytes repeated would otherwise be unpacked to megabytes for every frame.
#[test]
fn lz_unpacking_stops_at_what_a_picture_can_use() {
    let mut spec = Spec::new(16, 2);
    spec.unpack = u32::MAX;
    let mut d = Decoder::new(&build(&spec, &[])).unwrap();
    // 4 GiB declared; long chains of 273 bytes from the window's fill, far more than 32 pixels
    let mut data = vec![0x82, 0xFF, 0xFF, 0xFF, 0xFF, 0x34, 0x12, 0x78, 0x56];
    for _ in 0..40_000 {
        data.push(0);
        data.extend([0x00, 0x0F, 0xFF].repeat(8));
    }
    let start = std::time::Instant::now();
    for _ in 0..200 {
        d.decode(FULL, false, &data).unwrap();
    }
    assert_eq!(d.indices(), [0x20; 32]);
    // 87 MB of output for each frame without the stop: seconds, not milliseconds
    assert!(start.elapsed() < std::time::Duration::from_secs(2), "{:?}", start.elapsed());

    // the stop leaves room for the largest body a frame needs: every two pixels as a run-length
    // run of their own, five bytes for two pixels
    let (w, h) = (128, 32);
    let mut d = Decoder::new(&build(&Spec::new(w, h), &[])).unwrap();
    let body: Vec<u8> = (0..w * h / 2).flat_map(|i| [0x81, 0xFF, 0x81, i as u8, (i >> 8) as u8]).collect();
    // LZ packing with literals only: tags of 0xFF, eight bytes each
    let mut data = vec![0x83];
    data.extend((body.len() as u32).to_le_bytes());
    for chunk in body.chunks(8) {
        data.push(0xFF);
        data.extend_from_slice(chunk);
    }
    d.decode(Rect { left: 0, top: 0, right: w - 1, bottom: h - 1 }, false, &data).unwrap();
    let want: Vec<u8> = (0..w * h / 2).flat_map(|i| [i as u8, (i >> 8) as u8]).collect();
    assert!(d.indices() == want);
}

#[test]
fn palette_changes() {
    let mut d = flat();
    assert_eq!(d.palette()[0x40], [0x41; 3]);
    assert_eq!(d.palette()[255], [255; 3]);
    let mut palette = [0u8; 768];
    palette[0x40 * 3..][..3].copy_from_slice(&[63, 1, 32]);
    // a palette and nothing else
    d.decode(FULL, true, &palette_chunk(&palette)).unwrap();
    assert_eq!(d.palette()[0x40], [255, 4, 130]);
    assert_eq!(d.rgb24()[..3], [255, 4, 130]);
    assert_eq!(d.rgba8()[..4], [255, 4, 130, 255]);
    assert_eq!(d.indices(), [0x40; 32]);
    // every entry is replaced, whatever the first-index and count bytes say
    let mut chunk = palette_chunk(&[63; 768]);
    chunk[..2].copy_from_slice(&[4, 2]);
    d.decode(FULL, true, &chunk).unwrap();
    assert_eq!((d.palette()[0], d.palette()[255]), ([255; 3], [255; 3]));
    // components are 6 bits: higher bits are dropped
    d.decode(FULL, true, &palette_chunk(&[0x41; 768])).unwrap();
    assert_eq!(d.palette()[7], [4; 3]);
    assert_eq!(d.decode(FULL, true, &[0; 769]), Err(Error::Invalid("the palette is cut short")));
}

/// A frame whose rectangle is empty paints nothing, but its palette counts: the pictures after
/// it use it. ffmpeg was seen to do the same with such a frame.
#[test]
fn palette_of_a_frame_without_a_rectangle() {
    let spec = Spec::new(4, 2);
    let picture = |fill: u8| [vec![2], vec![fill; 8]].concat();
    let mut palette = [0u8; 768];
    palette[5 * 3..][..3].copy_from_slice(&[63, 0, 63]);
    palette[6 * 3..][..3].copy_from_slice(&[0, 63, 0]);
    let bytes = build(
        &spec,
        &[
            vec![Frame::video([0, 0, 3, 1], false, picture(5))],
            // right is one less than left: no pixels, a palette and a picture that is not painted
            vec![Frame::video([1, 1, 0, 1], true, [palette_chunk(&palette), picture(6)].concat())],
            vec![Frame::video([0, 0, 1, 0], false, picture(6)[..3].to_vec())],
        ],
    );
    assert_eq!(open(&bytes).unwrap().picture_rect(1), None);
    let pictures = decode_all(&bytes);
    // index 5 is dark grey in the header's palette
    assert_eq!(pictures[0], (vec![5; 8], [4u8; 24].to_vec()));
    assert_eq!(pictures[1], (vec![5; 8], [255, 0, 255].repeat(8)));
    assert_eq!(pictures[2].0, [6, 6, 5, 5, 5, 5, 5, 5]);
    assert_eq!(pictures[2].1[..9], [0, 255, 0, 0, 255, 0, 255, 0, 255]);

    let mut d = flat();
    d.decode_palette(&[]).unwrap();
    assert_eq!(d.palette()[0x40], [0x41; 3]);
    assert_eq!(d.decode_palette(&[0; 769]), Err(Error::Invalid("the palette is cut short")));
}

#[test]
fn partial_updates_and_empty_frames() {
    let mut d = flat();
    d.decode(Rect { left: 4, top: 1, right: 7, bottom: 1 }, false, &[2, 1, 2, 3, 4]).unwrap();
    assert_eq!(d.indices()[..20], [[0x40; 4]; 5].concat());
    assert_eq!(d.indices()[20..24], [1, 2, 3, 4]);
    let before = d.indices().to_vec();
    d.decode(FULL, false, &[]).unwrap();
    assert_eq!(d.indices(), before);
    assert_eq!((d.width(), d.height()), (16, 2));
}

#[test]
fn bad_frames_are_errors() {
    let mut d = flat();
    for rect in [Rect { right: 16, ..FULL }, Rect { bottom: 2, ..FULL }, Rect { left: 9, right: 8, ..FULL }, Rect { top: 1, bottom: 0, ..FULL }] {
        assert_eq!(d.decode(rect, false, &[2; 40]), Err(Error::Invalid("the frame lies outside the picture")), "{rect:?}");
    }
    assert_eq!(d.decode(FULL, false, &[2; 32]), Err(Error::Invalid("the frame data ends early")));
    assert_eq!(d.decode(FULL, false, &[1, 0x8F]), Err(Error::Invalid("the frame data ends early")));
    // 9 + 8 pixels in a line of 16
    assert_eq!(d.decode(FULL, false, &[1, 8, 7]), Err(Error::Invalid("a run crosses the end of its line")));
    assert_eq!(d.decode(FULL, false, &[3, 0x8F, 0xFF, 9, 1, 2]), Err(Error::Invalid("run-length data overruns its run")));
    assert_eq!(d.decode(FULL, false, &[4, 0, 0]), Err(Error::Unsupported("unknown rendering method")));
    // the header allows 1 MiB of LZ output
    assert_eq!(d.decode(FULL, false, &[0x82, 0, 0, 0x10, 0x01]), Err(Error::Invalid("an LZ-packed frame is larger than the unpack buffer")));
}

#[test]
fn bad_headers_are_errors() {
    let good = build(&Spec::new(16, 2), &[]);
    let with = |patch: &dyn Fn(&mut Vec<u8>)| {
        let mut h = good.clone();
        patch(&mut h);
        Decoder::new(&h).err()
    };
    assert_eq!(with(&|_| {}), None);
    assert_eq!(with(&|h| h.truncate(815)), Some(Error::Invalid("the header is shorter than 816 bytes")));
    assert_eq!(with(&|h| h[0] = 0), Some(Error::Invalid("not a VMD header")));
    assert_eq!(with(&|h| h[12..14].fill(0)), Some(Error::Invalid("the picture has no pixels")));
    assert_eq!(with(&|h| h[4] = 5), Some(Error::Unsupported("24-bit pictures")));
    assert_eq!(with(&|h| h[4] = 13), Some(Error::Unsupported("16-bit pictures")));
    assert_eq!(with(&|h| h[4] = 7), Some(Error::Unsupported("Indeo 3 video")));
    assert_eq!(with(&|h| h[24..28].copy_from_slice(b"IV32")), Some(Error::Unsupported("Indeo 3 video")));
    // 65 535 x 65 535 is refused before anything is allocated
    assert_eq!(with(&|h| h[12..16].fill(0xFF)), Some(Error::TooLarge(65535, 65535)));
}

#[test]
fn hostile_input_never_panics() {
    for (seed, w, h, frames) in [(11, 24, 10, 12), (12, 140, 3, 8)] {
        let m = movie(seed, w, h, frames);
        craft_media_testkit::hostile::hammer(&format!("vmd video seed {seed}"), &m.bytes, 20_000, |d| {
            let Ok(file) = open(d) else { return };
            let Ok(mut decoder) = Decoder::new(file.header_bytes()) else { return };
            for (i, frame) in file.frames.iter().enumerate() {
                let FrameKind::Video { new_palette, left, top, right, bottom } = frame.kind else { continue };
                let Ok(data) = file.read_frame(d, i) else { continue };
                // the rectangle as stored too: the decoder must refuse what does not fit
                let _ = decoder.decode(Rect { left, top, right, bottom }, new_palette, &data);
                if let Some(rect) = file.picture_rect(i) {
                    let _ = decoder.decode(rect.into(), new_palette, &data);
                }
                let _ = decoder.decode_palette(&data);
            }
            let _ = decoder.rgb24();
        });
    }
}

/// Real files, for whoever has some: every `.vmd` in the directory `CRAFT_MEDIA_VMD_SAMPLES`
/// names must decode to the pictures ffmpeg gets. Game files are not redistributable, so none
/// are in the repository and the test is skipped without the variable.
#[test]
fn real_files_match_ffmpeg() {
    let Some(dir) = std::env::var_os("CRAFT_MEDIA_VMD_SAMPLES").filter(|d| !d.is_empty()) else {
        craft_media_testkit::skip_test("vmd samples", "CRAFT_MEDIA_VMD_SAMPLES is not set");
        return;
    };
    let Some(ffmpeg) = craft_media_testkit::ffmpeg_or_skip("vmd video samples") else { return };
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("vmd")))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no .vmd files in CRAFT_MEDIA_VMD_SAMPLES");
    let mut pictures = 0u64;
    for path in &paths {
        let name = path.display();
        let source = std::fs::File::open(path).unwrap();
        let file = open(&source).unwrap();
        let mut decoder = Decoder::new(file.header_bytes()).unwrap();
        let mut child = Command::new(&ffmpeg)
            .args(["-v", "quiet", "-i"])
            .arg(path)
            // passthrough: one picture a video frame, none repeated to fill blocks that have no video
            .args(["-an", "-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut theirs_out = child.stdout.take().unwrap();
        let mut theirs = vec![0u8; decoder.width() * decoder.height() * 3];
        for (i, frame) in file.frames.iter().enumerate() {
            let FrameKind::Video { new_palette, .. } = frame.kind else { continue };
            // ffmpeg gives no picture for a frame without data
            if frame.size == 0 {
                continue;
            }
            let data = file.read_frame(&source, i).unwrap();
            // as the documented loop does: a frame without a rectangle may still bring a palette
            match file.picture_rect(i) {
                Some(rect) => decoder.decode(rect.into(), new_palette, &data),
                None if new_palette => decoder.decode_palette(&data),
                None => panic!("{name}: frame {i} lies outside the picture"),
            }
            .unwrap_or_else(|e| panic!("{name}: frame {i}: {e}"));
            theirs_out.read_exact(&mut theirs).unwrap_or_else(|e| panic!("{name}: ffmpeg has no picture for frame {i}: {e}"));
            let ours = decoder.rgb24();
            if theirs != ours {
                let wrong: Vec<usize> = (0..ours.len() / 3).filter(|&p| ours[p * 3..][..3] != theirs[p * 3..][..3]).collect();
                let (w, first) = (decoder.width(), wrong[0]);
                panic!(
                    "{name}: frame {i} (block {}) differs from ffmpeg in {} pixels, first at ({}, {}); {:?}, data starts {:02x?}",
                    frame.block,
                    wrong.len(),
                    first % w,
                    first / w,
                    frame.kind,
                    &data[..data.len().min(8)]
                );
            }
            pictures += 1;
        }
        assert_eq!(theirs_out.read(&mut theirs).unwrap(), 0, "{name}: ffmpeg has more pictures");
        // pictures that match but an ffmpeg that failed are no comparison
        let status = child.wait().unwrap();
        assert!(status.success(), "{name}: ffmpeg exited with {status}");
    }
    println!("{} files, {pictures} pictures match ffmpeg", paths.len());
}
