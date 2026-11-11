//! Truncated, oversized and hostile input: the reader returns an error within its `Limits` and
//! never panics.

use super::common;

use common::*;
use craft_doc::{Error, Limits, info, outline, renditions, sniff};

fn small() -> Limits {
    let mut l = Limits::default();
    l.max_json = 64 << 10;
    l.max_entry = 64 << 10;
    l.max_total = 256 << 10;
    l
}

fn limit_exceeded<T: std::fmt::Debug>(r: Result<T, Error>) -> bool {
    matches!(r, Err(Error::LimitExceeded(_)))
}

/// Every call on `bytes`; none may panic.
fn exercise(bytes: &[u8], limits: &Limits) {
    let _ = sniff(bytes);
    let _ = info(bytes, limits);
    let _ = renditions(bytes, limits);
    let _ = outline(bytes, limits);
}

fn samples() -> Vec<Vec<u8>> {
    let members = format!(r#""preview":{},"pdf":{}"#, inline("image/png", &png(2, 2, 3)), blob("application/pdf", 0, PDF.len()));
    let mut v4 = vectorcraft_json(4, &members).into_bytes();
    v4.extend(BLOB_MAGIC);
    v4.extend(PDF);
    vec![gzip(&v4), v4, pcraft(&png(3, 3, 1), &png(6, 6, 2)), ecproj(2).into_bytes()]
}

#[test]
fn a_gzip_bomb_stops_at_the_limits() {
    // 8 MiB of padding in the JSON, compressed to a few KiB.
    let pad = "x".repeat(8 << 20);
    let json = vectorcraft_json(4, &format!(r#""pad":"{pad}""#));
    let gz = gzip(json.as_bytes());
    assert!(gz.len() < 64 << 10);
    assert!(limit_exceeded(info(&gz, &small())));
    assert!(limit_exceeded(outline(&gz, &small())));
    assert!(limit_exceeded(renditions(&gz, &small())));
    // The data after the JSON counts against max_total as far as it is read.
    let mut tail = vectorcraft_json(4, &format!(r#""pdf":{}"#, blob("application/pdf", 8 << 20, 1))).into_bytes();
    tail.extend(BLOB_MAGIC);
    tail.extend(vec![0u8; (8 << 20) + 1]);
    let gz = gzip(&tail);
    assert!(limit_exceeded(renditions(&gz, &small())));
    let mut more = small();
    more.max_total = 9 << 20;
    assert_eq!(renditions(&gz, &more).unwrap()[0].bytes, [0]);
    // info stops at the marker and reads the JSON only.
    assert_eq!(info(&gz, &small()).unwrap().version, 4);
}

#[test]
fn a_comment_holding_an_end_signature_does_not_hide_the_archive() {
    let mut pc = pcraft(&png(1, 1, 0), &png(1, 1, 0));
    let comment = b"PK\x05\x06 not the end record";
    let n = pc.len();
    pc[n - 2..].copy_from_slice(&(comment.len() as u16).to_le_bytes());
    pc.extend(comment);
    assert_eq!(info(&pc, &small()).unwrap().version, 1);
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
}

#[test]
fn seeded_mutations_never_panic() {
    let mut rng = Rng(0x5eed);
    for s in samples() {
        for _ in 0..1500 {
            let mut d = s.clone();
            for _ in 0..1 + rng.below(4) {
                let at = rng.below(d.len());
                match rng.below(4) {
                    0 => d[at] ^= 1 << rng.below(8),
                    1 => d[at] = rng.next() as u8,
                    2 => {
                        let v = [0u32, 1, u32::MAX, 0x8000_0000][rng.below(4)];
                        let end = (at + 4).min(d.len());
                        d[at..end].copy_from_slice(&v.to_le_bytes()[..end - at]);
                    }
                    _ => {
                        let end = (at + 1 + rng.below(64)).min(d.len());
                        d.drain(at..end);
                        if d.is_empty() {
                            break;
                        }
                    }
                }
            }
            exercise(&d, &small());
        }
    }
}
