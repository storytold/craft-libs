//! The CC0 sample files (raw.pixls.us) pinned in `samples.json`: each decodes to exactly the
//! sensor samples the derivation verified against the black-box reference (`clean-room/
//! 03-verification.md`), recorded as the SHA-256 of the little-endian sample buffer, with the
//! levels, white balance and metadata the decoder read then.
//!
//! The files are not in the repository (5–11 MB each): `python scripts/fetch-crw-samples.py
//! <dir>` downloads and verifies them, and `CRAFT_CRW_SAMPLES=<dir>` points the test at them.
//! Without the variable the test prints that it skipped and passes; `CRAFT_CRW_REQUIRE_SAMPLES=1`
//! (CI) turns the skip into a failure.
//!
//! `CRAFT_CRW_ORACLE=<dir>` additionally compares every sample against `<name>.crw.tiff`
//! references next to the files (16-bit grey TIFFs of the whole sensor, as written by the
//! reference decoder's `unprocessed_raw -T`), the check the derivation used. Those references
//! are not distributable and live outside the repository.

use craft_crw::{Error, Limits, decode, probe};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct Sample {
    name: String,
    body: String,
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
fn manifest_is_well_formed() {
    let m = manifest();
    assert_eq!(m.len(), 11);
    for s in &m {
        assert!(s.name.ends_with(".crw") && !s.name.contains('/') && !s.name.contains('\\'), "{}", s.name);
        assert_eq!(s.sha256.len(), 64, "{}", s.name);
        if let Expect::Decoded(d) = &s.expect {
            assert_eq!(d.samples_sha256.len(), 64, "{}", s.name);
            assert!(matches!(d.bits, 10 | 12), "{}", s.name);
        } else {
            assert!(s.body.contains("G5"), "only the PowerShot G5 is refused: {}", s.name);
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

/// Minimal reader for the reference TIFFs: one 16-bit grey image, stripped, either byte order.
fn oracle_samples(bytes: &[u8]) -> (usize, usize, Vec<u16>) {
    let be = match &bytes[..4] {
        b"II*\0" => false,
        b"MM\0*" => true,
        other => panic!("not a TIFF: {other:?}"),
    };
    let u16_at = |at: usize| -> u16 {
        let b: [u8; 2] = bytes[at..at + 2].try_into().unwrap();
        if be { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) }
    };
    let u32_at = |at: usize| -> u32 {
        let b: [u8; 4] = bytes[at..at + 4].try_into().unwrap();
        if be { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) }
    };
    let ifd = u32_at(4) as usize;
    let n = u16_at(ifd) as usize;
    let (mut w, mut h, mut bits, mut spp) = (0usize, 0usize, 0u16, 1u16);
    let (mut offsets, mut counts) = (Vec::new(), Vec::new());
    for k in 0..n {
        let e = ifd + 2 + k * 12;
        let (tag, typ, cnt) = (u16_at(e), u16_at(e + 2), u32_at(e + 4) as usize);
        let size = match typ {
            3 => 2,
            4 => 4,
            _ => 1,
        };
        let values_at = if cnt * size <= 4 { e + 8 } else { u32_at(e + 8) as usize };
        let value = |i: usize| -> usize { if typ == 3 { u16_at(values_at + i * 2) as usize } else { u32_at(values_at + i * 4) as usize } };
        match tag {
            256 => w = value(0),
            257 => h = value(0),
            258 => bits = value(0) as u16,
            277 => spp = value(0) as u16,
            273 => offsets = (0..cnt).map(value).collect(),
            279 => counts = (0..cnt).map(value).collect(),
            _ => {}
        }
    }
    assert_eq!((bits, spp), (16, 1), "oracle TIFF is 16-bit grey");
    let mut data = Vec::with_capacity(w * h);
    for (&o, &c) in offsets.iter().zip(&counts) {
        let strip = &bytes[o..o + c];
        data.extend(strip.as_chunks::<2>().0.iter().map(|p| if be { u16::from_be_bytes(*p) } else { u16::from_le_bytes(*p) }));
    }
    data.truncate(w * h);
    (w, h, data)
}

#[test]
fn oracle_tiffs_match_every_sample() {
    let Some(dir) = std::env::var_os("CRAFT_CRW_ORACLE").map(PathBuf::from) else {
        eprintln!("CRAFT_CRW_ORACLE not set; skipping the reference-TIFF comparison");
        return;
    };
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir).expect("oracle directory").filter_map(|e| e.ok()).map(|e| e.path()).collect();
    entries.sort();
    let mut checked = 0;
    for path in entries.iter().filter(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("crw"))) {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        let reference: &Path = &path.with_extension("crw.tiff");
        if !reference.is_file() {
            eprintln!("{name}: no reference TIFF, skipped");
            continue;
        }
        let bytes = std::fs::read(path).expect("read crw");
        let img = match decode(&bytes, &Limits::default()) {
            Ok(img) => img,
            Err(Error::Unsupported(why)) if why.contains("G5") => {
                eprintln!("{name}: G5 refused as documented");
                continue;
            }
            Err(e) => panic!("{name}: {e}"),
        };
        let (w, h, oracle) = oracle_samples(&std::fs::read(reference).expect("read oracle tiff"));
        assert_eq!((img.header.width, img.header.height), (w, h), "{name}: dimensions");
        assert_eq!(oracle.len(), w * h, "{name}: oracle sample count");
        assert_eq!(img.samples.len(), w * h, "{name}: decoded sample count");
        let mismatches = img.samples.iter().zip(&oracle).filter(|(a, b)| a != b).count();
        assert_eq!(mismatches, 0, "{name}: {mismatches} mismatching samples");
        eprintln!("{name}: EXACT ({w}x{h})");
        checked += 1;
    }
    assert!(checked > 0, "no .crw files with references found in {}", dir.display());
}
