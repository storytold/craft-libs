//! The CC0 sample files (raw.pixls.us) pinned in `samples.json`: each decodes to exactly the
//! sensor samples the derivation verified against the black-box reference (`clean-room/
//! 03-verification.md`), recorded as the SHA-256 of the little-endian sample buffer, with the
//! levels, white balance and metadata the decoder read then.
//!
//! The files are not in the repository (5–11 MB each): `python scripts/fetch-crw-samples.py
//! <dir>` downloads and verifies them, and `CRAFT_CRW_SAMPLES=<dir>` points the test at them.
//! Without the variable the test prints that it skipped and passes; `CRAFT_CRW_REQUIRE_SAMPLES=1`
//! (CI) turns the skip into a failure.

use craft_crw::{Error, Limits, decode, probe};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Deserialize)]
struct Sample {
    name: String,
    sha256: String,
    expect: Expect,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Expect {
    Unsupported(String),
    Decoded(Decoded),
}

#[derive(Deserialize)]
struct Decoded {
    width: usize,
    height: usize,
    bits: u32,
    selector: u32,
    cfa: String,
    active: [usize; 4],
    model: String,
    exif_orientation: u8,
    capture_unix: i64,
    wb: Option<[f32; 3]>,
    black: [f32; 4],
    white: f32,
    samples_sha256: String,
}

fn manifest() -> Vec<Sample> {
    serde_json::from_str(include_str!("samples.json")).expect("samples.json parses")
}

fn hex(digest: impl AsRef<[u8]>) -> String {
    digest.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn samples_dir() -> Option<PathBuf> {
    match std::env::var_os("CRAFT_CRW_SAMPLES") {
        Some(dir) => Some(PathBuf::from(dir)),
        None if std::env::var_os("CRAFT_CRW_REQUIRE_SAMPLES").is_some_and(|v| v == "1") => {
            panic!("CRAFT_CRW_REQUIRE_SAMPLES=1 but CRAFT_CRW_SAMPLES is unset")
        }
        None => {
            eprintln!("CRAFT_CRW_SAMPLES not set; skipping the sample-file tests (scripts/fetch-crw-samples.py)");
            None
        }
    }
}

#[test]
fn sample_files_decode_to_the_verified_samples() {
    let Some(dir) = samples_dir() else { return };
    let mut checked = 0;
    for s in manifest() {
        let path = dir.join(&s.name);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(hex(Sha256::digest(&bytes)), s.sha256, "{}: not the pinned CC0 file", s.name);
        assert!(probe(&bytes), "{}: not recognised", s.name);
        let started = std::time::Instant::now();
        let result = decode(&bytes, &Limits::default());
        let took = started.elapsed();
        match (&s.expect, result) {
            (Expect::Unsupported(kind), Err(Error::Unsupported(why))) => {
                assert_eq!(kind, "unsupported", "{}", s.name);
                assert!(why.contains("G5"), "{}: {why}", s.name);
                eprintln!("{}: refused as documented ({why})", s.name);
            }
            (Expect::Unsupported(_), other) => panic!("{}: expected Unsupported, got {other:?}", s.name.clone()),
            (Expect::Decoded(d), Ok(img)) => {
                let h = &img.header;
                assert_eq!((h.width, h.height, h.bits, h.selector), (d.width, d.height, d.bits, d.selector), "{}", s.name);
                assert_eq!(h.cfa.name(), d.cfa, "{}", s.name);
                assert_eq!([h.active_area.x, h.active_area.y, h.active_area.width, h.active_area.height], d.active, "{}", s.name);
                assert_eq!(h.model.as_deref(), Some(d.model.as_str()), "{}", s.name);
                assert_eq!(h.exif_orientation, d.exif_orientation, "{}", s.name);
                assert_eq!(h.capture_unix, Some(d.capture_unix), "{}", s.name);
                match (h.wb_multipliers, d.wb) {
                    (None, None) => {}
                    (Some(got), Some(want)) => {
                        assert!(got.iter().zip(&want).all(|(a, b)| (a - b).abs() < 1e-4), "{}: wb {got:?} vs {want:?}", s.name)
                    }
                    (got, want) => panic!("{}: wb {got:?} vs {want:?}", s.name),
                }
                assert!(
                    img.black.values.iter().zip(&d.black).all(|(a, b)| (a - b).abs() < 0.01),
                    "{}: black {:?} vs {:?}",
                    s.name,
                    img.black.values,
                    d.black
                );
                assert_eq!(img.white, d.white, "{}", s.name);
                assert_eq!(img.samples.len(), d.width * d.height, "{}", s.name);
                let mut hasher = Sha256::new();
                for v in &img.samples {
                    hasher.update(v.to_le_bytes());
                }
                assert_eq!(hex(hasher.finalize()), d.samples_sha256, "{}: the decoded samples differ from the verified decode", s.name);
                eprintln!("{}: {}x{} {}-bit selector {} EXACT in {took:.0?}", s.name, h.width, h.height, h.bits, h.selector);
            }
            (Expect::Decoded(_), Err(e)) => panic!("{}: {e}", s.name),
        }
        checked += 1;
    }
    assert_eq!(checked, 11);
}
