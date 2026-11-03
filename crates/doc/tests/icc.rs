//! Synthetic PhotoCraft ICC payloads: the declared bytes are opaque, content-addressed blobs.
#![cfg(feature = "pcraft-pixels")]

mod common;

use common::{deflated, store, zip};
use craft_doc::{Error, Limits, pcraft_icc_profile};

/// Blob identity is over the decoded payload, not its compressed representation.
fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// The producer's streaming encoder omits a declared content size.
fn frame(bytes: &[u8]) -> Vec<u8> {
    ruzstd::encoding::compress_to_vec(bytes, ruzstd::encoding::CompressionLevel::Fastest)
}

/// A minimal checked envelope around the document chosen by each regression.
fn manifest(document: &str) -> String {
    format!(r#"{{"format_version":1,"generator":"synthetic","document":{document}}}"#)
}

/// A profile read needs no canvas or layer metadata.
fn named_manifest(hash: &str) -> String {
    manifest(&format!(r#"{{"icc_profile":"{hash}"}}"#))
}

/// Store a blob at its content address without assuming its bytes are a valid ICC profile.
fn bundle(payload: &[u8]) -> Vec<u8> {
    let name = craft_pcraft_format::blob_path(&hash(payload)).unwrap();
    zip(&[store("manifest.json", named_manifest(&hash(payload)).as_bytes()), store(&name, &frame(payload))])
}

#[test]
/// Native absence is missing or null; declared blobs remain opaque, including empty ones.
fn absent_and_null_are_none_but_declared_payloads_are_returned_without_semantic_checks() {
    for document in ["{}", r#"{"icc_profile":null}"#] {
        assert_eq!(pcraft_icc_profile(&zip(&[store("manifest.json", manifest(document).as_bytes())]), &Limits::default()), Ok(None));
    }
    for payload in [&b"not an ICC header"[..], &b""[..], &[0, 1, 2, 3][..]] {
        assert_eq!(pcraft_icc_profile(&bundle(payload), &Limits::default()), Ok(Some(payload.to_vec())));
    }
}

#[test]
/// Either selected ZIP entry may use the producer's supported outer compression method.
fn store_and_deflate_entries_use_the_same_bounded_blob_reader() {
    let payload = b"synthetic profile bytes";
    let h = hash(payload);
    let path = craft_pcraft_format::blob_path(&h).unwrap();
    let m = named_manifest(&h);
    let z = frame(payload);
    for archive in
        [zip(&[store("manifest.json", m.as_bytes()), deflated(&path, &z)]), zip(&[deflated("manifest.json", m.as_bytes()), store(&path, &z)])]
    {
        assert_eq!(pcraft_icc_profile(&archive, &Limits::default()), Ok(Some(payload.to_vec())));
    }
}

#[test]
/// Bad references and non-object documents are errors rather than an absent profile.
fn invalid_references_types_and_duplicate_profile_members_are_malformed() {
    let h = hash(b"p");
    for document in [
        r#"{"icc_profile":""}"#.to_owned(),
        format!(r#"{{"icc_profile":"{}"}}"#, h.to_uppercase()),
        r#"{"icc_profile":"../manifest.json"}"#.to_owned(),
        r#"{"icc_profile":3}"#.to_owned(),
        r#"{"icc_profile":[]}"#.to_owned(),
        r#"{"icc_profile":null,"icc_profile":null}"#.to_owned(),
        format!(r#"{{"icc_profile":"{h}","icc_profile":null}}"#),
        "[]".to_owned(),
        "null".to_owned(),
    ] {
        let archive = zip(&[store("manifest.json", manifest(&document).as_bytes())]);
        assert!(matches!(pcraft_icc_profile(&archive, &Limits::default()), Err(Error::Malformed(_))), "{document}");
    }
}

#[test]
/// A declared payload must exist, match its content address and occupy exactly one frame.
fn missing_wrong_hash_or_damaged_declared_blobs_are_errors() {
    let h = hash(b"wanted");
    let path = craft_pcraft_format::blob_path(&h).unwrap();
    let m = named_manifest(&h);
    let valid = frame(b"wanted");
    let mut trailing = valid.clone();
    trailing.push(0);
    let mut multiple = valid.clone();
    multiple.extend_from_slice(&frame(b"second"));
    for archive in [
        zip(&[store("manifest.json", m.as_bytes())]),
        zip(&[store("manifest.json", m.as_bytes()), store(&path, &frame(b"other"))]),
        zip(&[store("manifest.json", m.as_bytes()), store(&path, b"not zstd")]),
        zip(&[store("manifest.json", m.as_bytes()), store(&path, &valid[..valid.len() - 1])]),
        zip(&[store("manifest.json", m.as_bytes()), store(&path, &trailing)]),
        zip(&[store("manifest.json", m.as_bytes()), store(&path, &multiple)]),
    ] {
        assert!(matches!(pcraft_icc_profile(&archive, &Limits::default()), Err(Error::Malformed(_))));
    }
}

#[test]
/// Unused previews and tile payloads cannot prevent reading an independently valid profile.
fn unrelated_payloads_are_never_read_and_document_metadata_is_skipped() {
    let payload = b"profile";
    let h = hash(payload);
    let path = craft_pcraft_format::blob_path(&h).unwrap();
    let m = format!(
        r#"{{"format_version":1,"generator":"synthetic","thumbnail":"thumb.png","composite":"composite/preview.png",
        "document":{{"icc_profile":"{h}","layers":[{{"content":{{"surface":{{"tiles":[{{"hash":"invalid"}}]}}}}}}],"future":[{{"ignored":true}}]}}}}"#
    );
    let archive = zip(&[store("manifest.json", m.as_bytes()), store(&path, &frame(payload)), store("tiles/invalid.zst", b"broken tile")]);
    assert_eq!(pcraft_icc_profile(&archive, &Limits::default()), Ok(Some(payload.to_vec())));
}

#[test]
/// Accepted work is exactly the manifest plus selected frame plus decoded payload.
fn manifest_frame_and_decoded_profile_have_exact_total_and_entry_boundaries() {
    let payload = vec![7; 32 << 10];
    let h = hash(&payload);
    let path = craft_pcraft_format::blob_path(&h).unwrap();
    let m = named_manifest(&h);
    let z = frame(&payload);
    let archive = zip(&[store("manifest.json", m.as_bytes()), store(&path, &z)]);
    let mut limits = Limits::default();
    limits.max_json = m.len() as u64;
    limits.max_entry = payload.len() as u64;
    limits.max_total = (m.len() + z.len() + payload.len()) as u64;
    assert_eq!(pcraft_icc_profile(&archive, &limits), Ok(Some(payload.clone())));
    limits.max_total -= 1;
    assert!(matches!(pcraft_icc_profile(&archive, &limits), Err(Error::LimitExceeded(_))));
    limits.max_total += 1;
    limits.max_entry -= 1;
    assert!(matches!(pcraft_icc_profile(&archive, &limits), Err(Error::LimitExceeded(_))));
    limits.max_entry += 1;
    limits.max_json -= 1;
    assert!(matches!(pcraft_icc_profile(&archive, &limits), Err(Error::LimitExceeded(_))));
    let absent = manifest("{}");
    limits.max_json = absent.len() as u64;
    limits.max_total = absent.len() as u64;
    assert_eq!(pcraft_icc_profile(&zip(&[store("manifest.json", absent.as_bytes())]), &limits), Ok(None));
    limits.max_total -= 1;
    assert!(matches!(pcraft_icc_profile(&zip(&[store("manifest.json", absent.as_bytes())]), &limits), Err(Error::LimitExceeded(_))));
}

#[test]
/// The frame's ZIP size must fail admission even if its bytes would also fail decoding.
fn an_oversized_frame_is_refused_before_the_zstd_decoder() {
    let m = named_manifest(&hash(b"p"));
    let path = craft_pcraft_format::blob_path(&hash(b"p")).unwrap();
    let mut limits = Limits::default();
    limits.max_entry = m.len() as u64;
    let archive = zip(&[store("manifest.json", m.as_bytes()), store(&path, &vec![0; m.len() + 1])]);
    assert!(matches!(pcraft_icc_profile(&archive, &limits), Err(Error::LimitExceeded(_))));
}

#[test]
/// Skipping document values must not let a small typed head evade the caller's item cap.
fn structural_item_limits_include_skipped_fields_before_the_profile_head() {
    let archive = bundle(b"p");
    let mut limits = Limits::default();
    limits.max_items = 4; // root object, its two commas and document object
    assert_eq!(pcraft_icc_profile(&archive, &limits), Ok(Some(b"p".to_vec())));
    limits.max_items = 3;
    assert!(matches!(pcraft_icc_profile(&archive, &limits), Err(Error::LimitExceeded(_))));
    let m = manifest(r#"{"icc_profile":null,"ignored":[0,0,0,0,0]}"#);
    let archive = zip(&[store("manifest.json", m.as_bytes())]);
    limits.max_items = 4;
    assert!(matches!(pcraft_icc_profile(&archive, &limits), Err(Error::LimitExceeded(_))));
    limits.max_items = 0;
    assert!(matches!(pcraft_icc_profile(&zip(&[store("manifest.json", manifest("{}").as_bytes())]), &limits), Err(Error::LimitExceeded(_))));
}

#[test]
/// Absence of a profile cannot bypass kind, envelope, format-version or nesting checks.
fn versions_kinds_envelope_and_depth_are_checked_even_without_a_profile() {
    assert_eq!(pcraft_icc_profile(b"not a bundle", &Limits::default()), Err(Error::Unrecognized));
    assert!(matches!(pcraft_icc_profile(common::vectorcraft_json(4, "").as_bytes(), &Limits::default()), Err(Error::Unsupported(_))));
    let m = manifest("{}").replace("\"format_version\":1", "\"format_version\":2");
    assert!(matches!(pcraft_icc_profile(&zip(&[store("manifest.json", m.as_bytes())]), &Limits::default()), Err(Error::TooNew { .. })));
    for m in [
        r#"{"format_version":1,"generator":"a","generator":"b","document":{}}"#,
        r#"{"format_version":1,"generator":"a","document":{},"thumbnail":"elsewhere"}"#,
    ] {
        assert!(matches!(pcraft_icc_profile(&zip(&[store("manifest.json", m.as_bytes())]), &Limits::default()), Err(Error::Malformed(_))));
    }
    let m = manifest(r#"{"ignored":[[[0]]]}"#);
    let mut limits = Limits::default();
    limits.max_depth = 3;
    assert!(matches!(pcraft_icc_profile(&zip(&[store("manifest.json", m.as_bytes())]), &limits), Err(Error::LimitExceeded(_))));
}

#[test]
/// Header-window limits and streamed output admission remain effective without a size hint.
fn excessive_window_and_size_less_expansion_are_limits() {
    let payload = vec![0; 64 << 10];
    let h = hash(&payload);
    let path = craft_pcraft_format::blob_path(&h).unwrap();
    let m = named_manifest(&h);
    let z = frame(&payload);
    // Producer frames omit their content size; output admission must work without that hint.
    assert_eq!(z[4] & 0xc0, 0);
    let mut limits = Limits::default();
    limits.max_entry = 1024;
    assert!(matches!(pcraft_icc_profile(&zip(&[store("manifest.json", m.as_bytes()), store(&path, &z)]), &limits), Err(Error::LimitExceeded(_))));
    let mut large_window = z.clone();
    assert_eq!(large_window[4] & 0x20, 0);
    large_window[5] = 14 << 3; // 16 MiB
    assert!(matches!(
        pcraft_icc_profile(&zip(&[store("manifest.json", m.as_bytes()), store(&path, &large_window)]), &Limits::default()),
        Err(Error::LimitExceeded(_))
    ));
}

#[test]
/// Nonzero size hints permit early refusal but never replace checking actual decoded bytes.
fn declared_nonzero_content_sizes_are_checked_before_and_after_decoding() {
    let h = hash(b"p");
    let path = craft_pcraft_format::blob_path(&h).unwrap();
    let m = named_manifest(&h);
    // A single-segment frame declaring 4096 bytes, but holding one raw byte.
    let mut z = vec![0x28, 0xb5, 0x2f, 0xfd, 0x60];
    z.extend_from_slice(&(4096u16 - 256).to_le_bytes());
    z.extend_from_slice(&[9, 0, 0, b'p']);
    let archive = zip(&[store("manifest.json", m.as_bytes()), store(&path, &z)]);
    let mut limits = Limits::default();
    limits.max_entry = 1024;
    assert!(matches!(pcraft_icc_profile(&archive, &limits), Err(Error::LimitExceeded(_))));
    assert!(matches!(pcraft_icc_profile(&archive, &Limits::default()), Err(Error::Malformed(_))));
    let z = [0x28, 0xb5, 0x2f, 0xfd, 0x20, 1, 9, 0, 0, b'p'];
    assert_eq!(pcraft_icc_profile(&zip(&[store("manifest.json", m.as_bytes()), store(&path, &z)]), &limits), Ok(Some(b"p".to_vec())));
}

#[test]
/// A present zero size permits an empty payload, while an omitted size permits streamed data.
fn explicit_zero_content_sizes_match_only_empty_payloads() {
    // A two-byte FCS has an offset of 256, so zero uses one, four or eight bytes.
    for (descriptor, width, single) in [(0x80, 4, false), (0xc0, 8, false), (0x20, 1, true), (0xa0, 4, true), (0xe0, 8, true)] {
        for payload in [b"p".as_slice(), b"".as_slice()] {
            let h = hash(payload);
            let path = craft_pcraft_format::blob_path(&h).unwrap();
            let m = named_manifest(&h);
            let mut z = vec![0x28, 0xb5, 0x2f, 0xfd, descriptor];
            if !single {
                z.push(0); // 1 KiB window
            }
            z.extend(std::iter::repeat_n(0, width));
            z.extend_from_slice(&[if payload.is_empty() { 1 } else { 9 }, 0, 0]);
            z.extend_from_slice(payload);
            let result = pcraft_icc_profile(&zip(&[store("manifest.json", m.as_bytes()), store(&path, &z)]), &Limits::default());
            if payload.is_empty() {
                assert_eq!(result, Ok(Some(Vec::new())), "descriptor {descriptor:#x}");
            } else {
                assert!(matches!(result, Err(Error::Malformed(_))), "descriptor {descriptor:#x}: {result:?}");
            }
        }
    }
    assert_eq!(pcraft_icc_profile(&bundle(b"p"), &Limits::default()), Ok(Some(b"p".to_vec())), "unknown content size still streams");
}

#[test]
/// Damaged envelopes and frames fail through the public accessor without escaping a panic.
fn truncations_and_mutations_do_not_panic() {
    let archive = bundle(b"synthetic profile");
    for i in 0..archive.len() {
        assert!(pcraft_icc_profile(&archive[..i], &Limits::default()).is_err());
        let mut changed = archive.clone();
        changed[i] ^= 0xff;
        let _ = pcraft_icc_profile(&changed, &Limits::default());
    }
}
