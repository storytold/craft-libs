//! Decodes of the libjpeg-turbo test images (tests/data, see SOURCE.md there).

use craft_jpeg::{Decoded, Limits, Samples, decode};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn decode_clean(name: &str) -> Decoded {
    let img = decode(&fixture(name), &Limits::default(), None).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(!img.damaged, "{name}");
    img
}

fn decode_fixture(name: &str) -> Decoded {
    let img = decode_clean(name);
    assert_eq!((img.width, img.height, img.components), (227, 149, 3), "{name}");
    img
}

/// Mean and maximum absolute difference.
fn diff(a: impl Iterator<Item = f64>, b: impl Iterator<Item = f64>) -> (f64, f64) {
    let (mut sum, mut max, mut n) = (0.0, 0.0f64, 0usize);
    for (a, b) in a.zip(b) {
        let d = (a - b).abs();
        sum += d;
        max = max.max(d);
        n += 1;
    }
    (sum / n.max(1) as f64, max)
}

/// testimgari-huffman.jpg is testimgari.jpg transcoded by jpegtran: the same coefficients, only
/// the entropy coding differs, so the samples must be identical.
#[test]
fn arithmetic_coded_image_matches_its_huffman_transcode() {
    let huffman = decode_fixture("testimgari-huffman.jpg");
    let arithmetic = decode_fixture("testimgari.jpg");
    assert!(matches!(huffman.samples, Samples::U8(_)));
    assert!(huffman.samples == arithmetic.samples, "testimgari.jpg decodes differently from testimgari-huffman.jpg");
}

/// The arithmetic and progressive arithmetic transcodes jpegtran made of testorig.jpg and
/// testorig12.jpg hold their coefficients, so they decode to exactly the same samples.
#[test]
fn arithmetic_transcodes_match_their_huffman_sources() {
    for (source, transcodes) in [
        ("testorig.jpg", &["testorig-ari-progressive.jpg", "testorig-ari-progressive-restart.jpg"]),
        ("testorig12.jpg", &["testorig12-ari.jpg", "testorig12-ari-progressive.jpg"]),
    ] {
        let reference = decode_fixture(source);
        for name in transcodes {
            assert!(decode_fixture(name).samples == reference.samples, "{name} decodes differently from {source}");
        }
    }
}

/// libjpeg's arithmetic encoder drops the trailing zero bytes of a scan, so the white lower
/// part of whitetail-ari.jpg decodes from zeros supplied at the EOI marker. It must decode
/// whole and match its Huffman transcode.
#[test]
fn arithmetic_white_tail_decodes_from_implied_zeros() {
    let arithmetic = decode_clean("whitetail-ari.jpg");
    assert_eq!((arithmetic.width, arithmetic.height, arithmetic.components), (512, 512, 1));
    assert!(decode_clean("whitetail-huffman.jpg").samples == arithmetic.samples, "whitetail-ari.jpg decodes differently from whitetail-huffman.jpg");
}

/// The 8-bit path against zune-jpeg. Both use centered linear chroma upsampling; the residual is
/// float against integer IDCT and color conversion.
#[test]
fn baseline_image_matches_zune_jpeg() {
    let bytes = fixture("testorig.jpg");
    let mut zune = zune_jpeg::JpegDecoder::new(zune_jpeg::zune_core::bytestream::ZCursor::new(&bytes[..]));
    let reference = zune.decode().unwrap_or_else(|e| panic!("zune-jpeg: {e:?}"));
    let img = decode_fixture("testorig.jpg");
    let Samples::U8(px) = &img.samples else { panic!("testorig.jpg: expected 8-bit samples") };
    assert_eq!(px.len(), reference.len());
    let (mean, max) = diff(px.iter().map(|&v| f64::from(v)), reference.iter().map(|&v| f64::from(v)));
    eprintln!("testorig.jpg vs zune-jpeg: mean {mean:.3}, max {max}");
    assert!(mean < MAX_MEAN_8 && max <= MAX_ABS_8, "mean {mean}, max {max}");
}

// Observed: mean 0.52, max 3.
const MAX_MEAN_8: f64 = 0.75;
const MAX_ABS_8: f64 = 4.0;

/// Parse a binary PPM with 16-bit big-endian samples.
fn ppm16(bytes: &[u8]) -> (u32, u32, u32, Vec<u16>) {
    let mut fields = Vec::new();
    let mut at = 0;
    while fields.len() < 4 {
        while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        let start = at;
        while bytes.get(at).is_some_and(|b| !b.is_ascii_whitespace()) {
            at += 1;
        }
        fields.push(std::str::from_utf8(&bytes[start..at]).unwrap_or_default().to_owned());
    }
    assert_eq!(fields[0], "P6");
    let num = |s: &str| s.parse::<u32>().unwrap_or_else(|e| panic!("PPM header {s}: {e}"));
    let (w, h, maxval) = (num(&fields[1]), num(&fields[2]), num(&fields[3]));
    let data = &bytes[at + 1..];
    let px: Vec<u16> = data.as_chunks::<2>().0.iter().map(|&c| u16::from_be_bytes(c)).collect();
    assert_eq!(px.len(), (w * h * 3) as usize);
    (w, h, maxval, px)
}

/// testorig12.jpg against libjpeg-turbo's decode (testorig12.ppm, 12-bit RGB; SOURCE.md says how
/// it was made). libjpeg-turbo uses an integer IDCT and integer color conversion, craft-jpeg
/// floats; the comparison is on the 12-bit scale.
#[test]
fn twelve_bit_image_matches_libjpeg_turbo() {
    let (w, h, maxval, reference) = ppm16(&fixture("testorig12.ppm"));
    assert_eq!((w, h, maxval), (227, 149, 4095));
    let img = decode_fixture("testorig12.jpg");
    let Samples::U16(px) = &img.samples else { panic!("testorig12.jpg: expected 16-bit samples") };
    assert_eq!(px.len(), reference.len());
    let (mean, max) = diff(px.iter().map(|&v| f64::from(v) * 4095.0 / 65535.0), reference.iter().map(|&v| f64::from(v)));
    eprintln!("testorig12.jpg vs libjpeg-turbo (12-bit units): mean {mean:.3}, max {max:.2}");
    assert!(mean < MAX_MEAN_12 && max <= MAX_ABS_12, "mean {mean}, max {max}");
}

// Observed: mean 0.50, max 2.46 (of 4095).
const MAX_MEAN_12: f64 = 0.75;
const MAX_ABS_12: f64 = 4.0;
