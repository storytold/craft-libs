//! Deterministic mutation run over the test images and fuzz seeds: decoding corrupted input never panics and
//! stays within the caller's limits.

use craft_jpeg::{Limits, Samples, decode, frame};

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
