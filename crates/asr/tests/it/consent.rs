//! The consent gate: a refused or pending consent performs no download, and an acceptance is
//! pinned to the exact manifest.

use std::cell::Cell;
use std::io::Write;

use craft_asr::{Consent, Decision, DownloadError, ModelFile, ModelManifest, Transport, TransportError, download_with, sha256_hex};

struct FakeTransport {
    body: Vec<u8>,
    calls: Cell<usize>,
}

impl FakeTransport {
    fn new(body: &[u8]) -> Self {
        FakeTransport { body: body.to_vec(), calls: Cell::new(0) }
    }
}

impl Transport for FakeTransport {
    fn fetch(&self, _url: &str, sink: &mut dyn Write) -> Result<(), TransportError> {
        self.calls.set(self.calls.get() + 1);
        sink.write_all(&self.body).map_err(|error| TransportError::new(error.to_string()))
    }
}

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
fn a_refused_consent_downloads_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest = manifest(b"weights");
    let transport = FakeTransport::new(b"weights");
    let target = dir.path().join("model");

    let pending = Consent::new();
    assert!(matches!(download_with(&pending, &manifest, &target, &transport, &mut |_, _| {}), Err(DownloadError::ConsentRequired)));

    let mut declined = Consent::new();
    declined.decline();
    assert_eq!(declined.decision(), Decision::Declined);
    assert!(matches!(download_with(&declined, &manifest, &target, &transport, &mut |_, _| {}), Err(DownloadError::ConsentRequired)));

    assert_eq!(transport.calls.get(), 0, "neither case opened the transport");
    assert!(!target.exists(), "neither case created the model directory");
}

#[test]
fn an_accepted_manifest_downloads_verifies_and_installs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest = manifest(b"weights");
    let transport = FakeTransport::new(b"weights");
    let target = dir.path().join("model");
    let mut consent = Consent::new();
    let acceptance = consent.accept(&manifest).expect("accept");
    assert!(consent.permits(&manifest));
    assert_eq!(acceptance.revision, "1");

    let mut progress = Vec::new();
    download_with(&consent, &manifest, &target, &transport, &mut |done, total| progress.push((done, total))).expect("download");
    assert_eq!(transport.calls.get(), 1);
    assert_eq!(std::fs::read(target.join("model.bin")).expect("file"), b"weights");
    assert_eq!(progress.last(), Some(&(7, 7)));
    assert!(target.join("manifest.json").is_file());
}

#[test]
fn changing_the_manifest_after_consent_refuses_the_download() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest = manifest(b"weights");
    let mut consent = Consent::new();
    consent.accept(&manifest).expect("accept");
    let mut edited = manifest;
    edited.files[0].sha256 = sha256_hex(b"other");
    assert!(!consent.permits(&edited), "the fingerprint no longer matches");
    let transport = FakeTransport::new(b"weights");
    assert!(matches!(download_with(&consent, &edited, &dir.path().join("model"), &transport, &mut |_, _| {}), Err(DownloadError::ConsentRequired)));
    assert_eq!(transport.calls.get(), 0);
}
