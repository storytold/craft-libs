//! The disclosure shown before a model download and the state machine behind its consent.
//!
//! A download moves hundreds of megabytes from a third party and installs files a recognizer
//! later loads. The user sees [`Disclosure`] (source, licence, size and each file's digest)
//! first, and calls [`Consent::accept`] only when they agree. [`Consent::permits`] is true only
//! for the exact manifest that was accepted: any change to a file, URL, size, digest or licence
//! invalidates the old acceptance, so an edited manifest cannot inherit consent.
//!
//! [`Consent::download`] checks the gate before it takes any other step, so a refused or pending
//! consent never opens a connection or creates a directory.

use sha2::{Digest, Sha256};

use crate::download::{DownloadError, Transport, download_with};
use crate::manifest::{ManifestError, ModelFile, ModelManifest};

/// What the user decided about one manifest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Decision {
    /// Nothing was shown or answered yet.
    #[default]
    Pending,
    /// The user explicitly accepted this manifest.
    Accepted,
    /// The user refused; no download is attempted until a new accept.
    Declined,
}

/// The manifest a consent was granted for, and the fingerprint that pins it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Acceptance {
    pub id: String,
    pub revision: String,
    /// Hex SHA-256 over the manifest's identity, licence and every file's name, size, digest and
    /// URL.
    pub fingerprint: String,
}

/// One file as the disclosure shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDisclosure {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
    pub url: String,
}

/// Everything the user must be shown before a download: what it is, where it comes from, what it
/// costs in bytes, its licence, and the digest of every file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disclosure {
    pub id: String,
    pub name: String,
    pub repo: String,
    pub revision: String,
    pub license: String,
    pub source_url: String,
    pub total_bytes: u64,
    pub files: Vec<FileDisclosure>,
}

impl Disclosure {
    /// Build the disclosure for a manifest. A damaged manifest is refused before anything can be
    /// shown or accepted.
    pub fn from_manifest(manifest: &ModelManifest) -> Result<Self, ManifestError> {
        manifest.validate()?;
        Ok(Disclosure {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            repo: manifest.repo.clone(),
            revision: manifest.revision.clone(),
            license: manifest.license.clone(),
            source_url: manifest.source_url.clone(),
            total_bytes: manifest.total_bytes(),
            files: manifest
                .files
                .iter()
                .map(|file| FileDisclosure { name: file.name.clone(), bytes: file.bytes, sha256: file.sha256.clone(), url: file.url.clone() })
                .collect(),
        })
    }
}

/// The consent state for one model.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Consent {
    decision: Decision,
    acceptance: Option<Acceptance>,
}

impl Consent {
    pub fn new() -> Self {
        Consent::default()
    }

    /// What the user decided.
    pub fn decision(&self) -> Decision {
        self.decision
    }

    /// The manifest the user accepted, when there is one.
    pub fn acceptance(&self) -> Option<&Acceptance> {
        self.acceptance.as_ref()
    }

    /// Record an explicit accept for `manifest`. The manifest must be usable; a damaged one is
    /// an error and leaves the previous decision untouched.
    pub fn accept(&mut self, manifest: &ModelManifest) -> Result<Acceptance, ManifestError> {
        let fingerprint = fingerprint(manifest)?;
        let acceptance = Acceptance { id: manifest.id.clone(), revision: manifest.revision.clone(), fingerprint };
        self.acceptance = Some(acceptance.clone());
        self.decision = Decision::Accepted;
        Ok(acceptance)
    }

    /// Record a refusal. A later download is refused until another explicit accept.
    pub fn decline(&mut self) {
        self.decision = Decision::Declined;
        self.acceptance = None;
    }

    /// Withdraw a previous accept.
    pub fn revoke(&mut self) {
        self.decision = Decision::Pending;
        self.acceptance = None;
    }

    /// Whether this consent allows `manifest` to be downloaded. True only when the user accepted
    /// an identical manifest (same id, revision and every file's name, size, digest and URL).
    pub fn permits(&self, manifest: &ModelManifest) -> bool {
        let Some(acceptance) = &self.acceptance else { return false };
        if self.decision != Decision::Accepted || acceptance.id != manifest.id || acceptance.revision != manifest.revision {
            return false;
        }
        fingerprint(manifest).is_ok_and(|current| current == acceptance.fingerprint)
    }

    /// Download `manifest` into `dir` through `transport`, reporting `(done, total)` bytes.
    ///
    /// Returns [`DownloadError::ConsentRequired`] before touching the disk or the transport when
    /// the user has not accepted this exact manifest.
    pub fn download<T: Transport>(
        &self,
        manifest: &ModelManifest,
        dir: &std::path::Path,
        transport: &T,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), DownloadError> {
        download_with(self, manifest, dir, transport, progress)
    }
}

/// A hex SHA-256 over everything that makes this manifest the one the user agreed to. The
/// encoding is deterministic and versioned; it is not a file format.
pub fn fingerprint(manifest: &ModelManifest) -> Result<String, ManifestError> {
    manifest.validate()?;
    let mut hasher = Sha256::new();
    field(&mut hasher, "craft-asr-manifest-v1");
    field(&mut hasher, &manifest.id);
    field(&mut hasher, &manifest.revision);
    field(&mut hasher, &manifest.license);
    field(&mut hasher, &manifest.source_url);
    let mut files: Vec<&ModelFile> = manifest.files.iter().collect();
    files.sort_by(|a, b| a.name.cmp(&b.name));
    for file in files {
        field(&mut hasher, &file.name);
        hasher.update(file.bytes.to_le_bytes());
        field(&mut hasher, &file.sha256.to_ascii_lowercase());
        field(&mut hasher, &file.url);
    }
    let mut out = String::with_capacity(64);
    for byte in hasher.finalize() {
        out.push_str(&format!("{byte:02x}"));
    }
    Ok(out)
}

fn field(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::sha256_hex;

    fn manifest() -> ModelManifest {
        ModelManifest {
            id: "test-model".into(),
            name: "Test".into(),
            repo: "owner/test".into(),
            revision: "rev-1".into(),
            license: "MIT".into(),
            source_url: "https://huggingface.co/owner/test".into(),
            files: vec![
                ModelFile { name: "a.bin".into(), bytes: 3, sha256: sha256_hex(b"abc"), url: "https://huggingface.co/owner/test/a.bin".into() },
                ModelFile { name: "b.bin".into(), bytes: 3, sha256: sha256_hex(b"xyz"), url: "https://huggingface.co/owner/test/b.bin".into() },
            ],
        }
    }

    #[test]
    fn nothing_is_permitted_until_the_exact_manifest_is_accepted() {
        let manifest = manifest();
        let mut consent = Consent::new();
        assert_eq!(consent.decision(), Decision::Pending);
        assert!(!consent.permits(&manifest), "a pending consent permits nothing");
        let acceptance =
            consent.accept(&manifest).unwrap_or_else(|_| Acceptance { id: String::new(), revision: String::new(), fingerprint: String::new() });
        assert_eq!(consent.decision(), Decision::Accepted);
        assert_eq!(acceptance.id, manifest.id);
        assert_eq!(acceptance.revision, manifest.revision);
        assert!(consent.permits(&manifest));
        consent.decline();
        assert_eq!(consent.decision(), Decision::Declined);
        assert!(!consent.permits(&manifest), "a refusal revokes the acceptance");
    }

    #[test]
    fn any_change_to_the_manifest_invalidates_an_acceptance() {
        let original = manifest();
        let mut consent = Consent::new();
        consent.accept(&original).unwrap_or_else(|_| Acceptance { id: String::new(), revision: String::new(), fingerprint: String::new() });
        let mut changed = original.clone();
        changed.files[0].sha256 = sha256_hex(b"different");
        assert!(!consent.permits(&changed), "a changed digest");
        let mut changed = original.clone();
        changed.files[0].url = "https://huggingface.co/owner/test/other.bin".into();
        assert!(!consent.permits(&changed), "a changed source");
        let mut changed = original.clone();
        changed.files[0].bytes = 4;
        assert!(!consent.permits(&changed), "a changed size");
        let mut changed = original.clone();
        changed.revision = "rev-2".into();
        assert!(!consent.permits(&changed), "a changed revision");
        let mut changed = original.clone();
        changed.license = "Apache-2.0".into();
        assert!(!consent.permits(&changed), "a changed licence");
        let mut changed = original.clone();
        changed.source_url = "https://huggingface.co/owner/other".into();
        assert!(!consent.permits(&changed), "a changed source page");
        consent.revoke();
        assert!(!consent.permits(&original), "a revoked consent permits nothing");
    }

    #[test]
    fn a_damaged_manifest_cannot_be_accepted_or_disclosed() {
        let mut damaged = manifest();
        damaged.files[0].name = "../escape".into();
        let mut consent = Consent::new();
        assert!(consent.accept(&damaged).is_err());
        assert_eq!(consent.decision(), Decision::Pending, "a refused accept keeps the previous decision");
        assert!(Disclosure::from_manifest(&damaged).is_err());
        assert!(fingerprint(&damaged).is_err());
    }

    #[test]
    fn the_disclosure_lists_every_file_and_its_evidence() {
        let manifest = manifest();
        let disclosure = Disclosure::from_manifest(&manifest).unwrap_or_else(|_| Disclosure {
            id: String::new(),
            name: String::new(),
            repo: String::new(),
            revision: String::new(),
            license: String::new(),
            source_url: String::new(),
            total_bytes: 0,
            files: Vec::new(),
        });
        assert_eq!(disclosure.id, manifest.id);
        assert_eq!(disclosure.total_bytes, manifest.total_bytes());
        assert_eq!(disclosure.files.len(), manifest.files.len());
        assert_eq!(disclosure.files[0].sha256, manifest.files[0].sha256);
        assert_eq!(disclosure.files[1].bytes, 3);
    }
}
