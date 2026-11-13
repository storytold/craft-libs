//! `decode_webp_frames`: every frame of an animated WebP, composited, with its duration (#1198).

use craft_image_codecs::{
    AnimationFrame, ChannelLayout, CodecError, DecodeOptions, DecodeWarning, EncodeOptions, Format, Image, Limits, decode, decode_webp_frames,
    encode, webp_is_animated,
};

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

fn frames(bytes: &[u8]) -> Vec<AnimationFrame> {
    decode_webp_frames(bytes, &DecodeOptions::default()).unwrap()
}

fn solid(c: [u8; 4], w: u32, h: u32) -> Image {
    Image::from_u8(w, h, ChannelLayout::Rgba, c.repeat((w * h) as usize)).unwrap()
}

fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut c = id.to_vec();
    c.extend_from_slice(&(data.len() as u32).to_le_bytes());
    c.extend_from_slice(data);
    if data.len() % 2 == 1 {
        c.push(0);
    }
    c
}

fn u24(v: u32) -> [u8; 3] {
    let b = v.to_le_bytes();
    [b[0], b[1], b[2]]
}

/// One frame of [`animated`]: the picture, where it sits on the canvas (even offsets, as WebP
/// stores them halved), its duration in milliseconds, and whether it is alpha-blended over the
/// canvas rather than replacing it.
struct Part {
    image: Image,
    at: [u32; 2],
    duration_ms: u32,
    blend: bool,
}

fn part(c: [u8; 4], w: u32, h: u32, duration_ms: u32) -> Part {
    Part { image: solid(c, w, h), at: [0, 0], duration_ms, blend: false }
}

/// An animated WebP with a `w`x`h` canvas (transparent background) and one ANMF chunk per part,
/// each holding a lossless still from our own encoder.
fn animated(w: u32, h: u32, parts: &[Part]) -> Vec<u8> {
    let mut body = b"WEBP".to_vec();
    let mut vp8x = vec![0x02 | 0x10, 0, 0, 0]; // animation and alpha flags
    vp8x.extend_from_slice(&u24(w - 1));
    vp8x.extend_from_slice(&u24(h - 1));
    body.extend(chunk(b"VP8X", &vp8x));
    body.extend(chunk(b"ANIM", &[0, 0, 0, 0, 0, 0])); // background 0, loop forever
    for p in parts {
        let still = encode(&p.image, Format::WebP, &EncodeOptions::default()).unwrap();
        let at = still.windows(4).position(|w| w == b"VP8L").unwrap();
        let len = u32::from_le_bytes(still[at + 4..at + 8].try_into().unwrap()) as usize;
        let mut anmf = Vec::new();
        anmf.extend_from_slice(&u24(p.at[0] / 2));
        anmf.extend_from_slice(&u24(p.at[1] / 2));
        anmf.extend_from_slice(&u24(p.image.width() - 1));
        anmf.extend_from_slice(&u24(p.image.height() - 1));
        anmf.extend_from_slice(&u24(p.duration_ms));
        anmf.push(if p.blend { 0 } else { 0x02 }); // bit 1: do not blend
        anmf.extend(chunk(b"VP8L", &still[at + 8..at + 8 + len]));
        body.extend(chunk(b"ANMF", &anmf));
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}

fn pixel(f: &AnimationFrame, x: u32, y: u32) -> [u8; 4] {
    let rgba = f.image.to_rgba8();
    let i = ((y * f.image.width() + x) * 4) as usize;
    rgba[i..i + 4].try_into().unwrap()
}

#[test]
fn a_still_webp_gives_one_frame_equal_to_decode() {
    let still = encode(&solid(RED, 6, 4), Format::WebP, &EncodeOptions::default()).unwrap();
    assert!(!webp_is_animated(&still));
    let f = frames(&still);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].delay_ms, 0);
    assert_eq!(f[0].image, decode(&still).unwrap());
}

#[test]
fn a_one_frame_animation_is_not_animated() {
    let a = animated(6, 4, &[part(RED, 6, 4, 70)]);
    assert!(!webp_is_animated(&a));
    assert_eq!(frames(&a).len(), 1);
}

#[test]
fn every_frame_comes_back_in_order_with_its_duration() {
    let a = animated(6, 4, &[part(RED, 6, 4, 40), part(GREEN, 6, 4, 120), part(BLUE, 6, 4, 7)]);
    assert!(webp_is_animated(&a));
    let f = frames(&a);
    assert_eq!(f.iter().map(|f| f.delay_ms).collect::<Vec<_>>(), [40, 120, 7], "WebP durations are milliseconds");
    for (f, c) in f.iter().zip([RED, GREEN, BLUE]) {
        assert_eq!((f.image.width(), f.image.height()), (6, 4));
        assert_eq!(pixel(f, 0, 0), c);
        assert_eq!(pixel(f, 5, 3), c);
        assert_eq!(f.image.warnings, []);
    }
}

#[test]
fn a_smaller_frame_is_drawn_over_the_previous_one() {
    let small = Part { image: solid(BLUE, 2, 2), at: [2, 2], duration_ms: 50, blend: true };
    let f = frames(&animated(6, 4, &[part(RED, 6, 4, 50), small]));
    assert_eq!(f.len(), 2);
    // Alpha blending rounds: within one level.
    let near = |a: [u8; 4], b: [u8; 4]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 1);
    assert!(near(pixel(&f[1], 0, 0), RED), "outside the second frame the first shows through");
    assert!(near(pixel(&f[1], 2, 2), BLUE));
    assert!(near(pixel(&f[1], 3, 3), BLUE));
    assert!(near(pixel(&f[1], 4, 2), RED));
}

#[test]
fn a_damaged_later_frame_keeps_the_frames_before_it() {
    let mut a = animated(6, 4, &[part(RED, 6, 4, 50), part(GREEN, 6, 4, 50), part(BLUE, 6, 4, 50)]);
    // Break the last frame's VP8L signature byte; the container around it stays whole.
    let last = a.windows(4).rposition(|w| w == b"VP8L").unwrap();
    a[last + 8] = 0;
    let f = frames(&a);
    assert_eq!(f.len(), 2);
    assert_eq!(pixel(&f[0], 0, 0), RED);
    assert_eq!(pixel(&f[1], 0, 0), GREEN);
    assert_eq!(f[0].image.warnings, [DecodeWarning::Truncated { format: Format::WebP }]);
}

#[test]
fn every_prefix_of_an_animated_webp_returns() {
    let a = animated(5, 4, &[part(RED, 5, 4, 50), part(GREEN, 5, 4, 50), part(BLUE, 5, 4, 50)]);
    for n in 0..a.len() {
        let _ = decode_webp_frames(&a[..n], &DecodeOptions::default());
    }
}

#[test]
fn dimension_limits_apply() {
    let a = animated(4, 4, &[part(RED, 4, 4, 50), part(GREEN, 4, 4, 50)]);
    let opts = DecodeOptions { limits: Limits { max_width: 3, ..Limits::default() }, ..Default::default() };
    assert!(matches!(decode_webp_frames(&a, &opts), Err(CodecError::LimitExceeded(_))));
}

#[test]
fn the_byte_budget_covers_all_frames_together() {
    // One 4x4 RGBA frame is 64 bytes. The decoder's own canvas counts against the budget too, so
    // two frames need more than 128; ten frames of the same size must not fit where two do.
    let two = animated(4, 4, &[part(RED, 4, 4, 50), part(GREEN, 4, 4, 50)]);
    let ten: Vec<Part> = (0..10).map(|i| part(if i % 2 == 0 { RED } else { BLUE }, 4, 4, 50)).collect();
    let ten = animated(4, 4, &ten);
    let opts = DecodeOptions { limits: Limits { max_alloc: 256, ..Limits::default() }, ..Default::default() };
    assert_eq!(decode_webp_frames(&two, &opts).unwrap().len(), 2);
    assert!(matches!(decode_webp_frames(&ten, &opts), Err(CodecError::LimitExceeded(_))));
}
