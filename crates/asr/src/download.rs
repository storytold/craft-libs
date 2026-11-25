//! The verified model download loop.
//!
//! A download only starts after [`Consent::permits`] is true for the exact manifest being
//! installed; the check runs before a directory is created or a transport is called. Every file
//! is written beside its final name, bounded by the size the manifest records, checked against
//! that size and SHA-256, and only then renamed. A failed or interrupted download leaves no file
//! that looks installed and no provenance sidecar.
//!
//! The transport is a small trait so tests never touch the network. The `download` feature adds
//! `HttpTransport`, a bounded-timeout HTTPS client; the manifest only accepts `https://` URLs
//! on the model host it names.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::consent::Consent;
use crate::manifest::{ManifestError, ModelFile, ModelManifest, VerifyError, verify_file, verify_path, write_sidecar};

/// A transport failure: DNS, TLS, an HTTP status, or a body that stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportError {
    message: String,
}

impl TransportError {
    pub fn new(message: impl Into<String>) -> Self {
        let mut message = message.into();
        if message.chars().count() > 400 {
            message = message.chars().take(400).collect();
        }
        TransportError { message }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TransportError {}

/// Where a model file comes from. Implementations write the body to `sink`; the caller bounds
/// how many bytes the sink accepts.
pub trait Transport {
    fn fetch(&self, url: &str, sink: &mut dyn Write) -> Result<(), TransportError>;
}

/// Why a model download could not finish.
#[derive(Debug)]
pub enum DownloadError {
    /// The user has not accepted this exact manifest; nothing was requested or written.
    ConsentRequired,
    /// The manifest is damaged; nothing was requested.
    Manifest(ManifestError),
    /// A destination or temporary path would be written through a symlink.
    UnsafeDestination(String),
    /// The local disk refused us.
    Io(std::io::Error),
    /// The transport failed for one file.
    Transport { file: String, problem: TransportError },
    /// The body was larger than the manifest records.
    Oversized { file: String, expected: u64 },
    /// The body ended before the manifest's size.
    Short { file: String, expected: u64, got: u64 },
    /// The written bytes do not match the manifest's size or digest.
    Verify { file: String, problem: VerifyError },
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadError::ConsentRequired => f.write_str("the model download has not been accepted"),
            DownloadError::Manifest(error) => write!(f, "the model manifest is damaged: {error}"),
            DownloadError::UnsafeDestination(path) => write!(f, "refusing to write through {path}"),
            DownloadError::Io(error) => write!(f, "disk error: {error}"),
            DownloadError::Transport { file, problem } => write!(f, "{file}: {problem}"),
            DownloadError::Oversized { file, expected } => write!(f, "{file}: the body is larger than the manifest's {expected} bytes"),
            DownloadError::Short { file, expected, got } => write!(f, "{file}: got {got} bytes, the manifest says {expected}"),
            DownloadError::Verify { file, problem } => write!(f, "{file}: {problem}"),
        }
    }
}

impl std::error::Error for DownloadError {}

impl From<ManifestError> for DownloadError {
    fn from(error: ManifestError) -> Self {
        DownloadError::Manifest(error)
    }
}

impl From<std::io::Error> for DownloadError {
    fn from(error: std::io::Error) -> Self {
        DownloadError::Io(error)
    }
}

/// Download every missing file of `manifest` into `dir`, reporting `(done, total)` bytes as it
/// goes, then write the provenance sidecar.
///
/// The consent gate is checked first: a pending or declined consent returns
/// [`DownloadError::ConsentRequired`] before any filesystem or transport work.
pub fn download_with<T: Transport>(
    consent: &Consent,
    manifest: &ModelManifest,
    dir: &Path,
    transport: &T,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), DownloadError> {
    if !consent.permits(manifest) {
        return Err(DownloadError::ConsentRequired);
    }
    manifest.validate()?;
    if std::fs::symlink_metadata(dir).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(DownloadError::UnsafeDestination(format!("{} (a symlink)", dir.display())));
    }
    std::fs::create_dir_all(dir)?;
    let total = manifest.total_bytes();
    let mut done = 0u64;
    for file in &manifest.files {
        if verify_file(dir, file).is_ok() {
            done = done.saturating_add(file.bytes).min(total);
            progress(done, total);
            continue;
        }
        let name = file.safe_name().ok_or_else(|| ManifestError::UnsafeName(file.name.clone()))?;
        let destination = dir.join(name);
        if std::fs::symlink_metadata(&destination).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(DownloadError::UnsafeDestination(destination.display().to_string()));
        }
        let part = part_path(dir, name);
        // A stale temporary from an interrupted run is replaced, but a symlink there is removed
        // as a link (never followed) before the exclusive create.
        if std::fs::symlink_metadata(&part).is_ok() {
            std::fs::remove_file(&part)?;
        }
        let job = FileJob { transport, file, part: &part, destination: &destination, name, total };
        job.run(&mut done, progress)?;
    }
    write_sidecar(dir, manifest)?;
    progress(total, total);
    Ok(())
}

fn part_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.{}.part", std::process::id()))
}

/// One file of a download: fetch it into its temporary path, bound the body, verify it, rename.
struct FileJob<'a, T> {
    transport: &'a T,
    file: &'a ModelFile,
    part: &'a Path,
    destination: &'a Path,
    name: &'a str,
    total: u64,
}

impl<T: Transport> FileJob<'_, T> {
    fn run(self, done: &mut u64, progress: &mut dyn FnMut(u64, u64)) -> Result<(), DownloadError> {
        let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(self.part)?;
        let mut sink = BoundedWriter { file: &mut out, limit: self.file.bytes, written: 0, exceeded: false };
        let fetched = self.transport.fetch(&self.file.url, &mut sink);
        let (written, exceeded) = (sink.written, sink.exceeded);
        let flushed = out.flush();
        drop(out);
        if exceeded {
            let _ = std::fs::remove_file(self.part);
            return Err(DownloadError::Oversized { file: self.name.to_string(), expected: self.file.bytes });
        }
        if let Err(problem) = fetched {
            let _ = std::fs::remove_file(self.part);
            return Err(DownloadError::Transport { file: self.name.to_string(), problem });
        }
        if let Err(error) = flushed {
            let _ = std::fs::remove_file(self.part);
            return Err(DownloadError::Io(error));
        }
        if written != self.file.bytes {
            let _ = std::fs::remove_file(self.part);
            return Err(DownloadError::Short { file: self.name.to_string(), expected: self.file.bytes, got: written });
        }
        if let Err(problem) = verify_path(self.part, self.file) {
            let _ = std::fs::remove_file(self.part);
            return Err(DownloadError::Verify { file: self.name.to_string(), problem });
        }
        // Re-check the destination after the body: a symlink planted while it was being read is
        // refused rather than followed by the rename.
        if std::fs::symlink_metadata(self.destination).is_ok_and(|meta| meta.file_type().is_symlink()) {
            let _ = std::fs::remove_file(self.part);
            return Err(DownloadError::UnsafeDestination(self.destination.display().to_string()));
        }
        if let Err(error) = std::fs::rename(self.part, self.destination) {
            let _ = std::fs::remove_file(self.part);
            return Err(DownloadError::Io(error));
        }
        *done = done.saturating_add(self.file.bytes).min(self.total);
        progress(*done, self.total);
        Ok(())
    }
}

/// A sink that refuses a body larger than the manifest's recorded size instead of growing the
/// file, so a hostile or misconfigured host cannot fill the disk.
struct BoundedWriter<'a> {
    file: &'a mut std::fs::File,
    limit: u64,
    written: u64,
    exceeded: bool,
}

impl Write for BoundedWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if (buf.len() as u64) > self.limit.saturating_sub(self.written) {
            self.exceeded = true;
            return Err(std::io::Error::other("the model body is larger than the manifest says"));
        }
        let written = self.file.write(buf)?;
        self.written = self.written.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

/// The HTTPS transport for model downloads, with a stalled-host guard. It is a plain
/// [`Transport`]: the consent gate, the bounds and the verification live in [`download_with`],
/// not here.
#[cfg(all(feature = "download", not(target_arch = "wasm32")))]
pub struct HttpTransport {
    agent: ureq::Agent,
}

#[cfg(all(feature = "download", not(target_arch = "wasm32")))]
impl HttpTransport {
    pub fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(std::time::Duration::from_secs(20)))
            .timeout_recv_response(Some(std::time::Duration::from_secs(60)))
            // Each read resets; this is the stall guard in the middle of a body.
            .timeout_per_read(Some(std::time::Duration::from_secs(60)))
            // The whole body, which reads do not reset.
            .timeout_recv_body(Some(std::time::Duration::from_secs(60 * 60)))
            .build()
            .into();
        HttpTransport { agent }
    }
}

#[cfg(all(feature = "download", not(target_arch = "wasm32")))]
impl Default for HttpTransport {
    fn default() -> Self {
        HttpTransport::new()
    }
}

#[cfg(all(feature = "download", not(target_arch = "wasm32")))]
impl Transport for HttpTransport {
    fn fetch(&self, url: &str, sink: &mut dyn Write) -> Result<(), TransportError> {
        let mut response = self.agent.get(url).call().map_err(|error| TransportError::new(error.to_string()))?;
        if !response.status().is_success() {
            return Err(TransportError::new(format!("the server answered {}", response.status())));
        }
        let mut reader = response.body_mut().as_reader();
        std::io::copy(&mut reader, sink).map_err(|error| TransportError::new(error.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::sha256_hex;
    use std::cell::Cell;
    use std::collections::BTreeMap;

    struct FakeTransport {
        bodies: BTreeMap<String, Vec<u8>>,
        calls: Cell<usize>,
    }

    impl FakeTransport {
        fn new(bodies: &[(&str, &[u8])]) -> Self {
            FakeTransport { bodies: bodies.iter().map(|(url, body)| ((*url).to_string(), body.to_vec())).collect(), calls: Cell::new(0) }
        }
    }

    impl Transport for FakeTransport {
        fn fetch(&self, url: &str, sink: &mut dyn Write) -> Result<(), TransportError> {
            self.calls.set(self.calls.get() + 1);
            let body = self.bodies.get(url).ok_or_else(|| TransportError::new("the server answered 404"))?;
            sink.write_all(body).map_err(|error| TransportError::new(error.to_string()))
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

    fn accepted(manifest: &ModelManifest) -> Consent {
        let mut consent = Consent::new();
        let _ = consent.accept(manifest);
        consent
    }

    #[test]
    fn a_download_without_consent_opens_no_transport_and_creates_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manifest = manifest(b"weights");
        let transport = FakeTransport::new(&[("https://huggingface.co/owner/test/model.bin", b"weights")]);
        let target = dir.path().join("model");
        for consent in [Consent::new(), {
            let mut consent = Consent::new();
            consent.decline();
            consent
        }] {
            assert!(matches!(download_with(&consent, &manifest, &target, &transport, &mut |_, _| {}), Err(DownloadError::ConsentRequired)));
        }
        assert_eq!(transport.calls.get(), 0, "no request was made");
        assert!(!target.exists(), "no directory was created");
    }

    #[test]
    fn an_accepted_manifest_downloads_verifies_and_installs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manifest = manifest(b"weights");
        let consent = accepted(&manifest);
        let transport = FakeTransport::new(&[("https://huggingface.co/owner/test/model.bin", b"weights")]);
        let target = dir.path().join("model");
        let mut seen = Vec::new();
        download_with(&consent, &manifest, &target, &transport, &mut |done, total| seen.push((done, total))).unwrap_or(());
        assert_eq!(transport.calls.get(), 1);
        assert_eq!(std::fs::read(target.join("model.bin")).unwrap_or_default(), b"weights");
        assert_eq!(seen.last(), Some(&(7, 7)));
        assert!(target.join("manifest.json").is_file(), "the provenance sidecar is written");
        // A second run sees the verified file and downloads nothing.
        download_with(&consent, &manifest, &target, &transport, &mut |_, _| {}).unwrap_or(());
        assert_eq!(transport.calls.get(), 1);
    }

    #[test]
    fn an_edited_manifest_does_not_inherit_consent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manifest = manifest(b"weights");
        let consent = accepted(&manifest);
        let mut edited = manifest;
        edited.files[0].sha256 = sha256_hex(b"other");
        let transport = FakeTransport::new(&[]);
        assert!(matches!(
            download_with(&consent, &edited, &dir.path().join("model"), &transport, &mut |_, _| {}),
            Err(DownloadError::ConsentRequired)
        ));
        assert_eq!(transport.calls.get(), 0);
    }

    fn run_body(body: &[u8], server: &[u8]) -> Result<(), DownloadError> {
        let dir = tempfile::tempdir().expect("tempdir");
        let manifest = manifest(body);
        let consent = accepted(&manifest);
        let transport = FakeTransport::new(&[("https://huggingface.co/owner/test/model.bin", server)]);
        let target = dir.path().join("model");
        let result = download_with(&consent, &manifest, &target, &transport, &mut |_, _| {});
        let parts: Vec<_> = std::fs::read_dir(&target)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".part"))
            .collect();
        assert!(parts.is_empty(), "a failed download removes its temporary file");
        if result.is_err() {
            assert!(!target.join("model.bin").exists(), "a refused body is not installed");
            assert!(!target.join("manifest.json").exists(), "a refused body leaves no provenance");
        } else {
            assert_eq!(std::fs::read(target.join("model.bin")).unwrap_or_default(), body);
        }
        result
    }

    #[test]
    fn a_short_oversized_or_mismatched_body_is_refused_and_cleaned_up() {
        assert!(matches!(run_body(b"weights", b"weig"), Err(DownloadError::Short { .. })));
        assert!(matches!(run_body(b"weights", b"weights plus more"), Err(DownloadError::Oversized { .. })));
        assert!(matches!(run_body(b"weights", b"WEIGHTS"), Err(DownloadError::Verify { .. })));
        assert!(run_body(b"weights", b"weights").is_ok());
    }

    #[test]
    fn a_transport_failure_writes_no_sidecar() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manifest = manifest(b"weights");
        let consent = accepted(&manifest);
        let transport = FakeTransport::new(&[]);
        let target = dir.path().join("model");
        assert!(matches!(download_with(&consent, &manifest, &target, &transport, &mut |_, _| {}), Err(DownloadError::Transport { .. })));
        assert!(!target.join("manifest.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_planted_destination_symlink_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manifest = manifest(b"weights");
        let consent = accepted(&manifest);
        let transport = FakeTransport::new(&[("https://huggingface.co/owner/test/model.bin", b"weights")]);
        let target = dir.path().join("model");
        std::fs::create_dir_all(&target).unwrap_or(());
        std::fs::write(target.join("outside"), b"untouched").unwrap_or(());
        std::os::unix::fs::symlink(target.join("outside"), target.join("model.bin")).unwrap_or(());
        assert!(matches!(download_with(&consent, &manifest, &target, &transport, &mut |_, _| {}), Err(DownloadError::UnsafeDestination(_))));
        assert_eq!(std::fs::read(target.join("outside")).unwrap_or_default(), b"untouched");
        assert_eq!(transport.calls.get(), 0, "the link is refused before a request");
    }
}
