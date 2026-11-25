//! Manifest validation and verification through the public API.

use craft_asr::{
    ModelFile, ModelManifest, ModelState, WHISPER_TINY_ID, WHISPER_TINY_REVISION, check_dir, sha256_hex, verify_file, whisper_tiny, write_sidecar,
};

fn manifest(body: &[u8]) -> ModelManifest {
    ModelManifest {
        id: "test-model".into(),
        name: "Test".into(),
        repo: "owner/test".into(),
        revision: "1".into(),
        license: "MIT".into(),
        source_url: "https://huggingface.co/owner/test".into(),
        files: vec![ModelFile {
            name: "model.bin".into(),
            bytes: body.len() as u64,
            sha256: sha256_hex(body),
            url: "https://huggingface.co/owner/test/model.bin".into(),
        }],
    }
}

#[test]
fn the_shipped_manifest_validates_and_names_its_licence_and_source() {
    let manifest = whisper_tiny();
    assert_eq!(manifest.validate(), Ok(()));
    assert_eq!(manifest.id, WHISPER_TINY_ID);
    assert_eq!(manifest.revision, WHISPER_TINY_REVISION);
    assert_eq!(manifest.license, "Apache-2.0");
    assert!(manifest.source_url.starts_with("https://huggingface.co/"));
    assert_eq!(manifest.files.len(), 3);
}

#[test]
fn a_wrong_size_and_a_wrong_hash_are_both_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest = manifest(b"weights");
    std::fs::write(dir.path().join("model.bin"), b"weig").expect("write");
    let error = verify_file(dir.path(), &manifest.files[0]).expect_err("short file");
    assert!(error.to_string().contains("4 bytes"), "{error}");
    std::fs::write(dir.path().join("model.bin"), b"WEIGHTS").expect("write");
    let error = verify_file(dir.path(), &manifest.files[0]).expect_err("wrong digest");
    assert!(error.to_string().contains("SHA-256"), "{error}");
    std::fs::write(dir.path().join("model.bin"), b"weights").expect("write");
    assert_eq!(verify_file(dir.path(), &manifest.files[0]), Ok(()));
}

#[test]
fn a_directory_reports_absent_incomplete_corrupt_then_ready() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest = manifest(b"weights");
    let model = dir.path().join("model");
    assert_eq!(check_dir(&model, &manifest).state, ModelState::Absent);
    std::fs::create_dir_all(&model).expect("mkdir");
    assert_eq!(check_dir(&model, &manifest).state, ModelState::Incomplete);
    std::fs::write(model.join("model.bin"), b"WEIGHTS").expect("write");
    assert_eq!(check_dir(&model, &manifest).state, ModelState::Corrupt);
    std::fs::write(model.join("model.bin"), b"weights").expect("write");
    assert_eq!(check_dir(&model, &manifest).state, ModelState::Ready);
}

#[test]
fn a_manifest_with_a_path_or_a_plain_http_source_is_refused() {
    let mut escaping = manifest(b"weights");
    escaping.files[0].name = "../escape".into();
    assert!(escaping.validate().is_err());

    let mut plain = manifest(b"weights");
    plain.files[0].url = "http://huggingface.co/owner/test/model.bin".into();
    assert!(plain.validate().is_err());
}

#[test]
fn the_sidecar_round_trips_the_manifest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest = whisper_tiny();
    let path = write_sidecar(dir.path(), &manifest).expect("sidecar");
    let back: ModelManifest = serde_json::from_slice(&std::fs::read(path).expect("read")).expect("parse");
    assert_eq!(back, manifest);
}
