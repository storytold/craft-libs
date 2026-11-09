//! Deterministic mutation run over the test images and fuzz seeds: decoding corrupted input never panics and
//! stays within the caller's limits.

use craft_jpeg::{Bands, Limits, Samples, decode, frame};

/// Inputs and mutation counts. The 227×149 test images take tens of milliseconds each to decode
/// in a debug build; the small fuzz seeds (SOF9 with restarts, 12-bit SOF9, SOF10, 12-bit SOF1)
/// take microseconds and get most of the cases.
const INPUTS: [(&str, usize); 8] = [
    ("tests/data/testorig.jpg", 12),
    ("tests/data/testimgari.jpg", 12),
    ("tests/data/testimgari-huffman.jpg", 12),
    ("tests/data/testorig12.jpg", 12),
    ("fuzz/corpus/decode/sof9-restart.jpg", 75),
    ("fuzz/corpus/decode/sof9-12bit.jpg", 75),
    ("fuzz/corpus/decode/sof10-progressive.jpg", 75),
    ("fuzz/corpus/decode/sof1-12bit.jpg", 75),
];

/// Room for the 227×149 fixtures, too little for a mutated frame header that grows them much.
fn test_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_pixels = 1 << 16;
    limits.max_alloc = 1 << 20;
    limits
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn byte(&mut self) -> u8 {
        self.next() as u8
    }
}

/// Offset of the first SOFn segment's body, if any.
fn sof_body(b: &[u8]) -> Option<usize> {
    (2..b.len().saturating_sub(1))
        .find(|&i| b[i] == 0xFF && matches!(b[i + 1], 0xC0..=0xCF) && !matches!(b[i + 1], 0xC4 | 0xC8 | 0xCC))
        .map(|i| i + 4)
}

fn mutate(rng: &mut Rng, original: &[u8]) -> Vec<u8> {
    let mut b = original.to_vec();
    match rng.below(6) {
        0 => b.truncate(rng.below(original.len())),
        1 => {
            for _ in 0..1 + rng.below(8) {
                let at = rng.below(b.len());
                b[at] ^= 1 << rng.below(8);
            }
        }
        2 => {
            for _ in 0..1 + rng.below(4) {
                let at = rng.below(b.len());
                b[at] = rng.byte();
            }
        }
        3 => {
            // Frame header fields: precision, height, width, component count and the
            // per-component sampling and table selectors.
            if let Some(body) = sof_body(&b) {
                for _ in 0..1 + rng.below(3) {
                    let at = body + rng.below(15);
                    if let Some(v) = b.get_mut(at) {
                        *v = rng.byte();
                    }
                }
            }
        }
        4 => {
            let at = rng.below(b.len());
            let len = rng.below(64).min(b.len() - at);
            b.drain(at..at + len);
        }
        _ => {
            // Repeat a span, e.g. a marker segment or part of the entropy-coded data.
            let at = rng.below(b.len());
            let len = rng.below(256).min(b.len() - at);
            let span = b[at..at + len].to_vec();
            let to = rng.below(b.len());
            b.splice(to..to, span);
        }
    }
    b
}

#[test]
fn mutated_streams_never_panic_and_stay_within_limits() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let limits = test_limits();
    let (mut ok, mut errors) = (0, 0);
    for (name, cases) in INPUTS {
        let path = format!("{}/{name}", env!("CARGO_MANIFEST_DIR"));
        let original = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert!(decode(&original, &limits, None).is_ok(), "{name} must decode within the test limits");
        for case in 0..cases {
            let bytes = mutate(&mut rng, &original);
            let _ = frame(&bytes);
            match decode(&bytes, &limits, None) {
                Ok(img) => {
                    let pixels = u64::from(img.width) * u64::from(img.height);
                    let (len, width) = match &img.samples {
                        Samples::U8(v) => (v.len() as u64, 1),
                        Samples::U16(v) => (v.len() as u64, 2),
                    };
                    assert!(pixels <= limits.max_pixels, "{name} case {case}: {pixels} pixels");
                    assert!(len * width <= limits.max_alloc, "{name} case {case}: {} bytes", len * width);
                    assert_eq!(len, pixels * u64::from(img.components), "{name} case {case}");
                    ok += 1;
                }
                Err(_) => errors += 1,
            }
        }
    }
    eprintln!("{ok} decoded, {errors} rejected");
    // A mutation run that only produces errors would no longer reach the entropy decoders.
    assert!(ok > 50 && errors > 50, "{ok} decoded, {errors} rejected");
}

/// Every band read in order, and (when the restart markers make more than band 0 a seek point)
/// read again from each seek point; `None` when `Bands::new` rejects the stream. Once it accepts
/// one, damage is reported, never an error: a failed read or seek fails the test.
fn read_bands(bytes: &[u8], limits: &Limits) -> Option<(Vec<u8>, Option<Vec<u8>>)> {
    let mut bands = Bands::new(bytes, limits, None).ok()?;
    assert!(bands.reader_bytes() <= limits.max_alloc);
    assert!(u64::from(bands.width()) * u64::from(bands.height()) <= limits.max_pixels);
    let mut reader = bands.reader();
    let mut in_order = Vec::new();
    while let Some(band) = reader.next_band().expect("a read after Bands::new accepted the stream") {
        in_order.extend_from_slice(band.samples);
    }
    assert_eq!(in_order.len(), bands.width() as usize * bands.height() as usize * usize::from(bands.components()));
    bands.find_seek_points();
    let points: Vec<usize> = bands.seek_points().collect();
    if points.len() < 2 {
        return Some((in_order, None));
    }
    let mut sought = vec![0u8; in_order.len()];
    let band_bytes = bands.band_bytes();
    let mut reader = bands.reader();
    for (k, &p) in points.iter().enumerate().rev() {
        reader.seek(p).expect("a seek to an advertised seek point");
        for _ in p..points.get(k + 1).copied().unwrap_or(bands.bands()) {
            let band = reader.next_band().expect("a read after a seek").expect("a band after a seek");
            let at = band.index * band_bytes;
            sought[at..at + band.samples.len()].copy_from_slice(band.samples);
        }
    }
    Some((in_order, Some(sought)))
}

/// A 4:2:0 baseline stream with a restart interval at every line of MCUs, from `jpeg-encoder`.
fn restart_stream() -> Vec<u8> {
    let (w, h) = (48u16, 40u16);
    let px: Vec<u8> = (0..usize::from(w) * usize::from(h)).flat_map(|i| [(i * 7) as u8, (i / 3) as u8, (i % 97) as u8]).collect();
    let mut out = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut out, 80);
    enc.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_2_0);
    enc.set_restart_interval(3);
    enc.encode(&px, w, h, jpeg_encoder::ColorType::Rgb).unwrap();
    out
}

/// The band decoder on the same kind of mutations: it never panics, stays within the limits,
/// and wherever seek points survive, reading from them gives the samples reading in order does.
#[test]
fn mutated_streams_never_panic_in_bands() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let limits = test_limits();
    let fixture = |name: &str| std::fs::read(format!("{}/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let inputs = [(fixture("tests/data/testorig.jpg"), 40), (fixture("tests/data/testimgari-huffman.jpg"), 40), (restart_stream(), 400)];
    let (mut ok, mut errors, mut sought) = (0, 0, 0);
    for (original, cases) in &inputs {
        assert!(read_bands(original, &limits).is_some());
        for case in 0..*cases {
            let bytes = mutate(&mut rng, original);
            match read_bands(&bytes, &limits) {
                Some((in_order, Some(by_seeking))) => {
                    assert!(in_order == by_seeking, "case {case}: seeking changes the samples");
                    sought += 1;
                    ok += 1;
                }
                Some(_) => ok += 1,
                None => errors += 1,
            }
        }
    }
    eprintln!("{ok} decoded ({sought} also by seeking), {errors} rejected");
    assert!(ok > 50 && errors > 20 && sought > 50, "{ok} decoded ({sought} by seeking), {errors} rejected");
}
