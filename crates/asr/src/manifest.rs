//! The model manifest and its verification.
//!
//! Model weights are never committed or shipped. The repository holds what a model *is* (its id,
//! revision, files, byte sizes, SHA-256 digests, licence and source URL); the user either accepts
//! a download after the disclosure in [`crate::consent`] or places the files by hand in a model
//! directory.
//!
//! A manifest is user-writable, so every field is checked before it is used: file names are one
//! safe path component, sizes are positive and bounded, digests are SHA-256, and URLs are
//! `https://` on the model host this crate names. Verification checks the size first and hashes
//! only a file whose length matches, so a truncated or replaced file is refused without reading
//! it twice.
//!
//! The `whisper_tiny` manifest describes `openai/whisper-tiny` at a pinned Hugging Face
//! revision; the digests were measured from that revision. The model is Apache-2.0, and the
//! architecture is described in Radford et al., *Robust Speech Recognition via Large-Scale Weak
//! Supervision* ([arXiv:2212.04356](https://arxiv.org/abs/2212.04356)).

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The provenance sidecar's file name inside a model directory.
pub const SIDECAR: &str = "manifest.json";
/// The id of the model this crate ships a manifest for.
pub const WHISPER_TINY_ID: &str = "whisper-tiny";
/// The `openai/whisper-tiny` revision the sizes and hashes below belong to.
pub const WHISPER_TINY_REVISION: &str = "169d4a4341b33bc18d8881c4b69c2e104e1cc0af";
/// The largest file a manifest entry may claim (1 GiB): a hand-edited manifest must not ask for
/// an allocation or a download the disk cannot hold.
pub const MAX_FILE_BYTES: u64 = 1 << 30;
/// The most files one model may list: enough for weights, config and tokenizer, far short of a
/// directory listing.
pub const MAX_FILES: usize = 64;
/// The longest id, name, repo, revision or licence string a manifest may carry.
pub const MAX_FIELD_CHARS: usize = 256;
/// The hosts a model file or its source page may come from. A manifest (or a sidecar a user can
/// edit) is checked before a URL is used: HTTPS only, and only the model hub this crate names.
pub const ALLOWED_MODEL_HOSTS: &[&str] = &["huggingface.co", "hf.co"];

/// Whether `url` is an `https://` URL on an allowlisted model host. Anything else (plain HTTP,
/// another host, a `file:`/`data:` scheme, a userinfo trick) is refused before a request or a
/// link is used.
pub fn model_url_allowed(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    if rest.chars().any(|c| c.is_control() || c.is_whitespace() || c == '\\') {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // `user@host` and `host:port`: only the actual host counts.
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host).trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }
    ALLOWED_MODEL_HOSTS.iter().any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
}

/// Why a manifest cannot be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestError {
    /// The id is empty or contains anything but letters, digits, `-` and `_`.
    InvalidId(String),
    /// A required string is empty.
    EmptyField { field: &'static str },
    /// A string is longer than [`MAX_FIELD_CHARS`].
    FieldTooLong { field: &'static str, max: usize },
    /// A model must list at least one file.
    NoFiles,
    /// The manifest lists more than [`MAX_FILES`] files.
    TooManyFiles { count: usize, max: usize },
    /// A file name is not one safe path component.
    UnsafeName(String),
    /// A file digest is not 64 hexadecimal digits.
    BadHash { name: String },
    /// A file size is zero or above [`MAX_FILE_BYTES`].
    BadSize { name: String, bytes: u64, max: u64 },
    /// A model or source URL is not HTTPS on an allowed host.
    BadUrl { field: String },
    /// A file name is listed twice.
    Duplicate { name: String },
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::InvalidId(id) => write!(f, "`{id}` is not a model id"),
            ManifestError::EmptyField { field } => write!(f, "the manifest's {field} is empty"),
            ManifestError::FieldTooLong { field, max } => write!(f, "the manifest's {field} is longer than {max} characters"),
            ManifestError::NoFiles => f.write_str("the manifest lists no files"),
            ManifestError::TooManyFiles { count, max } => write!(f, "the manifest lists {count} files, above {max}"),
            ManifestError::UnsafeName(name) => write!(f, "unsafe model file name `{name}`"),
            ManifestError::BadHash { name } => write!(f, "`{name}` has no SHA-256"),
            ManifestError::BadSize { name, bytes, max } => write!(f, "`{name}` has an implausible size of {bytes} bytes (max {max})"),
            ManifestError::BadUrl { field } => write!(f, "the manifest's {field} is not HTTPS on an allowlisted host"),
            ManifestError::Duplicate { name } => write!(f, "`{name}` is listed twice"),
        }
    }
}

impl std::error::Error for ManifestError {}

/// One file of a model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelFile {
    /// A plain file name inside the model directory (never a path).
    pub name: String,
    /// The exact size in bytes.
    pub bytes: u64,
    /// Lower-case hex SHA-256 of the file's contents.
    pub sha256: String,
    /// Where the file is downloaded from.
    pub url: String,
}

impl ModelFile {
    /// The name when it is a single, safe path component; `None` for `..`, separators, an
    /// absolute path, a hidden file or an over-long name.
    pub fn safe_name(&self) -> Option<&str> {
        let name = self.name.as_str();
        let ok = !name.is_empty()
            && name.chars().count() <= 128
            && !name.starts_with('.')
            && !name.contains(['/', '\\', ':'])
            && !name.chars().any(|c| c.is_control());
        ok.then_some(name)
    }

    /// Whether the recorded digest is shaped like a SHA-256 (64 hex digits).
    pub fn has_valid_hash(&self) -> bool {
        self.sha256.len() == 64 && self.sha256.chars().all(|c| c.is_ascii_hexdigit())
    }
}

/// Everything needed to recognise, verify and fetch a model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelManifest {
    pub id: String,
    pub name: String,
    /// The upstream repository (`owner/name`).
    pub repo: String,
    /// The exact revision the files and hashes belong to.
    pub revision: String,
    /// The model's licence, as an SPDX identifier where one exists.
    pub license: String,
    /// Where the model card and licence can be read.
    pub source_url: String,
    pub files: Vec<ModelFile>,
}

impl ModelManifest {
    /// The total download size, saturating instead of overflowing on a hostile manifest.
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.bytes.min(MAX_FILE_BYTES)).fold(0, u64::saturating_add)
    }

    pub fn file(&self, name: &str) -> Option<&ModelFile> {
        self.files.iter().find(|file| file.name == name)
    }

    /// Every field is usable: the id is plain, strings are bounded, each entry names one safe
    /// file with a plausible size and digest, URLs stay on the model host, and no name repeats.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.id.is_empty() || !self.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(ManifestError::InvalidId(self.id.clone()));
        }
        for (field, value) in
            [("name", self.name.as_str()), ("repo", self.repo.as_str()), ("revision", self.revision.as_str()), ("license", self.license.as_str())]
        {
            if value.trim().is_empty() {
                return Err(ManifestError::EmptyField { field });
            }
            if value.chars().count() > MAX_FIELD_CHARS || value.chars().any(char::is_control) {
                return Err(ManifestError::FieldTooLong { field, max: MAX_FIELD_CHARS });
            }
        }
        if !model_url_allowed(&self.source_url) {
            return Err(ManifestError::BadUrl { field: "source URL".into() });
        }
        if self.files.is_empty() {
            return Err(ManifestError::NoFiles);
        }
        if self.files.len() > MAX_FILES {
            return Err(ManifestError::TooManyFiles { count: self.files.len(), max: MAX_FILES });
        }
        for (index, file) in self.files.iter().enumerate() {
            if file.safe_name().is_none() {
                return Err(ManifestError::UnsafeName(file.name.clone()));
            }
            if !file.has_valid_hash() {
                return Err(ManifestError::BadHash { name: file.name.clone() });
            }
            if file.bytes == 0 || file.bytes > MAX_FILE_BYTES {
                return Err(ManifestError::BadSize { name: file.name.clone(), bytes: file.bytes, max: MAX_FILE_BYTES });
            }
            if !model_url_allowed(&file.url) {
                return Err(ManifestError::BadUrl { field: format!("URL of {}", file.name) });
            }
            if self.files[..index].iter().any(|other| other.name.eq_ignore_ascii_case(&file.name)) {
                return Err(ManifestError::Duplicate { name: file.name.clone() });
            }
        }
        Ok(())
    }
}

/// The manifest of the model this crate documents: `openai/whisper-tiny`, Apache-2.0,
/// multilingual, 151 MB of weights. The digests were measured from the pinned revision in
/// [`WHISPER_TINY_REVISION`].
pub fn whisper_tiny() -> ModelManifest {
    let base = format!("https://huggingface.co/openai/whisper-tiny/resolve/{WHISPER_TINY_REVISION}");
    let file =
        |name: &str, bytes: u64, sha256: &str| ModelFile { name: name.to_string(), bytes, sha256: sha256.to_string(), url: format!("{base}/{name}") };
    ModelManifest {
        id: WHISPER_TINY_ID.to_string(),
        name: "Whisper Tiny".to_string(),
        repo: "openai/whisper-tiny".to_string(),
        revision: WHISPER_TINY_REVISION.to_string(),
        license: "Apache-2.0".to_string(),
        source_url: "https://huggingface.co/openai/whisper-tiny".to_string(),
        files: vec![
            file("model.safetensors", 151_061_672, "7ebd0e69e78190ffe1438491fa05cc1f5c1aa3a4c4db3bc1723adbb551ea2395"),
            file("config.json", 1_983, "ffdccec4f3211f4c63310f2b7098f309fe70f3952cedc5e4d11e43f5b2379b98"),
            file("tokenizer.json", 2_480_466, "27fc476bfe7f17299480be2273fc0608e4d5a99aba2ab5dec5374b4482d1a566"),
        ],
    }
}

/// The model's state after looking at a directory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelState {
    /// Every listed file is present with its recorded size and digest.
    Ready,
    /// Nothing there.
    #[default]
    Absent,
    /// The directory exists but files are missing.
    Incomplete,
    /// A file is there with the wrong size or digest, or the manifest is damaged.
    Corrupt,
}

impl ModelState {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelState::Ready => "ready",
            ModelState::Absent => "absent",
            ModelState::Incomplete => "incomplete",
            ModelState::Corrupt => "corrupt",
        }
    }
}

/// What was found, and why when it is not [`ModelState::Ready`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelStatus {
    pub state: ModelState,
    pub dir: Option<PathBuf>,
    pub problem: Option<String>,
}

impl ModelStatus {
    /// A message a dialog and a status line can show.
    pub fn summary(&self) -> String {
        match (&self.state, &self.problem) {
            (ModelState::Ready, _) => "Model installed".to_string(),
            (ModelState::Absent, _) => "Model not installed".to_string(),
            (state, Some(problem)) => format!("Model {}: {problem}", state.as_str()),
            (state, None) => format!("Model {}", state.as_str()),
        }
    }
}

/// Why a file or directory does not match its manifest entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyError {
    /// The entry does not name a safe file.
    UnsafeName(String),
    /// The file is not there.
    Missing { name: String },
    /// The path is not a regular file.
    NotAFile { name: String },
    /// The file's length is not the recorded length.
    Size { name: String, expected: u64, got: u64 },
    /// The file changed while it was being read.
    Changed { name: String },
    /// The file's SHA-256 is not the recorded digest.
    Hash { name: String },
    /// Reading the file failed.
    Io { name: String, problem: String },
    /// The caller cancelled the check.
    Cancelled,
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::UnsafeName(name) => write!(f, "unsafe model file name `{name}`"),
            VerifyError::Missing { name } => write!(f, "{name} is missing"),
            VerifyError::NotAFile { name } => write!(f, "{name} is not a file"),
            VerifyError::Size { name, expected, got } => write!(f, "{name} is {got} bytes, the manifest says {expected}"),
            VerifyError::Changed { name } => write!(f, "{name} changed while it was read"),
            VerifyError::Hash { name } => write!(f, "{name} does not match its SHA-256"),
            VerifyError::Io { name, problem } => write!(f, "{name}: {problem}"),
            VerifyError::Cancelled => f.write_str("the check was cancelled"),
        }
    }
}

impl std::error::Error for VerifyError {}

/// Lower-case hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Stream a file through SHA-256. Returns the length and the digest.
pub fn sha256_file(path: &Path) -> std::io::Result<(u64, String)> {
    sha256_file_until(path, &std::sync::atomic::AtomicBool::new(false))?.ok_or_else(|| std::io::Error::other("hashing was cancelled"))
}

/// Stream a file through SHA-256, stopping between blocks when `cancel` is set. `Ok(None)` means
/// the caller cancelled; a background check uses this so closing a dialog does not leave a
/// thread hashing 151 MB.
pub fn sha256_file_until(path: &Path, cancel: &std::sync::atomic::AtomicBool) -> std::io::Result<Option<(u64, String)>> {
    use std::sync::atomic::Ordering;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut len = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        len = len.saturating_add(read as u64);
        hasher.update(buf.get(..read).unwrap_or_default());
    }
    Ok(Some((len, hex(&hasher.finalize()))))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Check one file at `path` against `file`: length first, then SHA-256.
pub fn verify_path(path: &Path, file: &ModelFile) -> Result<(), VerifyError> {
    verify_path_until(path, file, &std::sync::atomic::AtomicBool::new(false))
}

fn verify_path_until(path: &Path, file: &ModelFile, cancel: &std::sync::atomic::AtomicBool) -> Result<(), VerifyError> {
    let name = file.name.clone();
    let meta = std::fs::metadata(path).map_err(|_| VerifyError::Missing { name: name.clone() })?;
    if !meta.is_file() {
        return Err(VerifyError::NotAFile { name });
    }
    if meta.len() != file.bytes {
        return Err(VerifyError::Size { name, expected: file.bytes, got: meta.len() });
    }
    let Some((len, hash)) = sha256_file_until(path, cancel).map_err(|error| VerifyError::Io { name: name.clone(), problem: error.to_string() })?
    else {
        return Err(VerifyError::Cancelled);
    };
    if len != file.bytes {
        return Err(VerifyError::Changed { name });
    }
    if !hash.eq_ignore_ascii_case(&file.sha256) {
        return Err(VerifyError::Hash { name });
    }
    Ok(())
}

/// Check one file inside `dir` against its manifest entry, stopping when `cancel` is set.
pub fn verify_file_until(dir: &Path, file: &ModelFile, cancel: &std::sync::atomic::AtomicBool) -> Result<(), VerifyError> {
    let name = file.safe_name().ok_or_else(|| VerifyError::UnsafeName(file.name.clone()))?;
    verify_path_until(&dir.join(name), file, cancel)
}

/// Check one file inside `dir` against its manifest entry.
pub fn verify_file(dir: &Path, file: &ModelFile) -> Result<(), VerifyError> {
    verify_file_until(dir, file, &std::sync::atomic::AtomicBool::new(false))
}

/// Check a model directory (usually `<root>/<model id>`), hashing every file.
pub fn check_dir(dir: &Path, manifest: &ModelManifest) -> ModelStatus {
    check_impl(dir, manifest, true)
}

/// Check a model directory without hashing: presence only, cheap enough for a UI frame or a show
/// start. Size and digest are checked when a model is installed or downloaded.
pub fn check_dir_files(dir: &Path, manifest: &ModelManifest) -> ModelStatus {
    check_impl(dir, manifest, false)
}

fn check_impl(dir: &Path, manifest: &ModelManifest, hash: bool) -> ModelStatus {
    if manifest.validate().is_err() {
        return ModelStatus { state: ModelState::Corrupt, dir: Some(dir.to_path_buf()), problem: Some("the manifest is damaged".into()) };
    }
    if !dir.is_dir() {
        return ModelStatus { state: ModelState::Absent, dir: None, problem: None };
    }
    let missing: Vec<&str> =
        manifest.files.iter().filter(|file| file.safe_name().is_none_or(|name| !dir.join(name).is_file())).map(|file| file.name.as_str()).collect();
    if !missing.is_empty() {
        return ModelStatus { state: ModelState::Incomplete, dir: Some(dir.to_path_buf()), problem: Some(format!("missing {}", missing.join(", "))) };
    }
    if !hash {
        return ModelStatus { state: ModelState::Ready, dir: Some(dir.to_path_buf()), problem: None };
    }
    for file in &manifest.files {
        if let Err(problem) = verify_file(dir, file) {
            return ModelStatus { state: ModelState::Corrupt, dir: Some(dir.to_path_buf()), problem: Some(problem.to_string()) };
        }
    }
    ModelStatus { state: ModelState::Ready, dir: Some(dir.to_path_buf()), problem: None }
}

/// Check a model directory, hashing every file, stopping when `cancel` is set. `None` means the
/// caller cancelled before the hashes finished.
pub fn check_dir_until(dir: &Path, manifest: &ModelManifest, cancel: &std::sync::atomic::AtomicBool) -> Option<ModelStatus> {
    use std::sync::atomic::Ordering;
    if manifest.validate().is_err() {
        return Some(ModelStatus { state: ModelState::Corrupt, dir: Some(dir.to_path_buf()), problem: Some("the manifest is damaged".into()) });
    }
    if !dir.is_dir() {
        return Some(ModelStatus { state: ModelState::Absent, dir: None, problem: None });
    }
    let missing: Vec<&str> =
        manifest.files.iter().filter(|file| file.safe_name().is_none_or(|name| !dir.join(name).is_file())).map(|file| file.name.as_str()).collect();
    if !missing.is_empty() {
        return Some(ModelStatus {
            state: ModelState::Incomplete,
            dir: Some(dir.to_path_buf()),
            problem: Some(format!("missing {}", missing.join(", "))),
        });
    }
    for file in &manifest.files {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        match verify_file_until(dir, file, cancel) {
            Ok(()) => {}
            Err(VerifyError::Cancelled) => return None,
            Err(problem) => {
                return Some(ModelStatus { state: ModelState::Corrupt, dir: Some(dir.to_path_buf()), problem: Some(problem.to_string()) });
            }
        }
    }
    Some(ModelStatus { state: ModelState::Ready, dir: Some(dir.to_path_buf()), problem: None })
}

/// Write the provenance sidecar (`manifest.json`) into a model directory.
///
/// The directory is user-writable and may be a folder the user picked, so the temporary file is
/// created exclusively (never through a pre-planted symlink), and an existing symlink at the
/// final name is refused rather than followed.
pub fn write_sidecar(dir: &Path, manifest: &ModelManifest) -> std::io::Result<PathBuf> {
    use std::io::Write;
    manifest.validate().map_err(std::io::Error::other)?;
    std::fs::create_dir_all(dir)?;
    let path = dir.join(SIDECAR);
    if std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(std::io::Error::other("refusing to write through the sidecar symlink"));
    }
    let json = serde_json::to_vec_pretty(manifest).map_err(std::io::Error::other)?;
    let tmp = dir.join(format!("manifest.json.{}.tmp", std::process::id()));
    if std::fs::symlink_metadata(&tmp).is_ok() {
        return Err(std::io::Error::other("refusing to reuse the sidecar temporary file"));
    }
    let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    out.write_all(&json)?;
    out.sync_all()?;
    drop(out);
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!("craft-asr-{}-{name}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap_or(());
        d
    }

    fn tiny_manifest(files: &[(&str, &[u8])]) -> ModelManifest {
        ModelManifest {
            id: "test-model".into(),
            name: "Test".into(),
            repo: "owner/test".into(),
            revision: "0".into(),
            license: "MIT".into(),
            source_url: "https://huggingface.co/owner/test".into(),
            files: files
                .iter()
                .map(|(name, bytes)| ModelFile {
                    name: (*name).into(),
                    bytes: bytes.len() as u64,
                    sha256: sha256_hex(bytes),
                    url: format!("https://huggingface.co/owner/test/{name}"),
                })
                .collect(),
        }
    }

    #[test]
    fn the_whisper_manifest_is_well_formed_and_matches_its_evidence() {
        let manifest = whisper_tiny();
        assert_eq!(manifest.validate(), Ok(()));
        assert_eq!(manifest.id, WHISPER_TINY_ID);
        assert_eq!(manifest.license, "Apache-2.0");
        assert_eq!(manifest.revision, WHISPER_TINY_REVISION);
        assert_eq!(manifest.total_bytes(), 153_544_121);
        let weights = manifest.file("model.safetensors").expect("weights entry");
        assert_eq!(weights.bytes, 151_061_672);
        assert_eq!(weights.sha256, "7ebd0e69e78190ffe1438491fa05cc1f5c1aa3a4c4db3bc1723adbb551ea2395");
        assert!(weights.url.ends_with("/model.safetensors") && weights.url.contains(WHISPER_TINY_REVISION));
        assert_eq!(manifest.file("config.json").map(|file| file.bytes), Some(1_983));
        assert_eq!(manifest.file("tokenizer.json").map(|file| file.bytes), Some(2_480_466));
    }

    #[test]
    fn hashing_matches_known_digests() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        let d = dir("hash");
        let path = d.join("a.bin");
        std::fs::write(&path, b"abc").unwrap_or(());
        let (len, hash) = sha256_file(&path).unwrap_or((0, String::new()));
        assert_eq!(len, 3);
        assert_eq!(hash, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn a_wrong_size_is_refused_before_a_digest_is_compared() {
        let manifest = tiny_manifest(&[("a.bin", b"hello")]);
        let d = dir("size");
        std::fs::write(d.join("a.bin"), b"hell").unwrap_or(());
        let error = verify_file(&d, &manifest.files[0]).unwrap_err();
        assert!(matches!(error, VerifyError::Size { expected: 5, got: 4, .. }), "{error}");
        std::fs::write(d.join("a.bin"), b"hello there").unwrap_or(());
        assert!(matches!(verify_file(&d, &manifest.files[0]), Err(VerifyError::Size { .. })));
    }

    #[test]
    fn a_wrong_digest_of_the_right_size_is_refused() {
        let manifest = tiny_manifest(&[("a.bin", b"hello")]);
        let d = dir("digest");
        std::fs::write(d.join("a.bin"), b"HELLO").unwrap_or(());
        assert!(matches!(verify_file(&d, &manifest.files[0]), Err(VerifyError::Hash { .. })));
        std::fs::write(d.join("a.bin"), b"hello").unwrap_or(());
        assert_eq!(verify_file(&d, &manifest.files[0]), Ok(()));
    }

    #[test]
    fn a_directory_is_absent_incomplete_corrupt_or_ready() {
        let manifest = tiny_manifest(&[("a.bin", b"hello"), ("b.bin", b"world")]);
        let d = dir("states");
        let model = d.join(&manifest.id);
        assert_eq!(check_dir(&model, &manifest).state, ModelState::Absent);
        std::fs::create_dir_all(&model).unwrap_or(());
        let missing = check_dir(&model, &manifest);
        assert_eq!(missing.state, ModelState::Incomplete);
        assert!(missing.problem.as_deref().is_some_and(|problem| problem.contains("a.bin")));
        std::fs::write(model.join("a.bin"), b"hello").unwrap_or(());
        std::fs::write(model.join("b.bin"), b"world").unwrap_or(());
        assert_eq!(check_dir(&model, &manifest).state, ModelState::Ready);
        assert_eq!(check_dir_files(&model, &manifest).state, ModelState::Ready);
        std::fs::write(model.join("b.bin"), b"World").unwrap_or(());
        let corrupt = check_dir(&model, &manifest);
        assert_eq!(corrupt.state, ModelState::Corrupt);
        assert!(corrupt.summary().contains("SHA-256"));
    }

    #[test]
    fn unsafe_names_and_damaged_manifests_are_rejected() {
        for name in ["../escape", "a/b", "C:evil", "", ".hidden", "a\\b"] {
            let mut manifest = tiny_manifest(&[("a.bin", b"x")]);
            manifest.files[0].name = name.into();
            assert!(manifest.validate().is_err(), "{name}");
            assert!(manifest.files[0].safe_name().is_none(), "{name}");
        }
        let mut manifest = tiny_manifest(&[("a.bin", b"x")]);
        manifest.files[0].sha256 = "nothex".into();
        assert!(matches!(manifest.validate(), Err(ManifestError::BadHash { .. })));
        let mut manifest = tiny_manifest(&[("a.bin", b"x")]);
        manifest.files.push(manifest.files[0].clone());
        assert!(matches!(manifest.validate(), Err(ManifestError::Duplicate { .. })));
        let mut manifest = tiny_manifest(&[("a.bin", b"x")]);
        manifest.files.clear();
        assert_eq!(manifest.validate(), Err(ManifestError::NoFiles));
        let mut too_many = tiny_manifest(&[("a.bin", b"x")]);
        for index in 0..MAX_FILES {
            too_many.files.push(ModelFile { name: format!("f{index}"), ..too_many.files[0].clone() });
        }
        assert!(matches!(too_many.validate(), Err(ManifestError::TooManyFiles { .. })));
        let d = dir("bad-manifest");
        assert_eq!(check_dir(&d, &manifest).state, ModelState::Corrupt, "a manifest with no files is damaged");
    }

    #[test]
    fn only_https_urls_on_the_model_hub_are_accepted() {
        for ok in [
            "https://huggingface.co/openai/whisper-tiny/resolve/abc/model.safetensors",
            "https://cdn-lfs.huggingface.co/repos/x",
            "https://HF.CO/x",
            "https://huggingface.co:443/x",
        ] {
            assert!(model_url_allowed(ok), "{ok}");
        }
        for bad in [
            "http://huggingface.co/x",
            "https://example.com/x",
            "https://huggingface.co.evil.example/x",
            "https://user@evil.example/x",
            "file:///etc/passwd",
            "https://",
            "https://huggingface.co\\evil",
        ] {
            assert!(!model_url_allowed(bad), "{bad}");
        }
        let mut manifest = tiny_manifest(&[("a.bin", b"x")]);
        manifest.files[0].url = "http://huggingface.co/a.bin".into();
        assert!(matches!(manifest.validate(), Err(ManifestError::BadUrl { .. })), "a plain-HTTP model file is refused");
        let mut manifest = tiny_manifest(&[("a.bin", b"x")]);
        manifest.source_url = "https://example.com/licence".into();
        assert!(matches!(manifest.validate(), Err(ManifestError::BadUrl { .. })), "a source page off the hub is refused");
    }

    #[test]
    fn a_cancelled_hash_check_stops_between_blocks() {
        let manifest = tiny_manifest(&[("a.bin", b"hello")]);
        let d = dir("cancel");
        let model = d.join(&manifest.id);
        std::fs::create_dir_all(&model).unwrap_or(());
        std::fs::write(model.join("a.bin"), b"hello").unwrap_or(());
        let cancel = std::sync::atomic::AtomicBool::new(true);
        assert_eq!(check_dir_until(&model, &manifest, &cancel), None, "a cancelled check reports nothing");
        let cancel = std::sync::atomic::AtomicBool::new(false);
        assert_eq!(check_dir_until(&model, &manifest, &cancel).map(|status| status.state), Some(ModelState::Ready));
    }

    #[test]
    fn the_sidecar_is_written_atomically_and_parses_back() {
        let manifest = whisper_tiny();
        let d = dir("sidecar");
        let path = write_sidecar(&d, &manifest).unwrap_or_else(|_| d.join("missing"));
        assert_eq!(path.file_name().and_then(|name| name.to_str()), Some(SIDECAR));
        let back: ModelManifest = serde_json::from_slice(&std::fs::read(&path).unwrap_or_default()).unwrap_or_else(|_| tiny_manifest(&[]));
        assert_eq!(back, manifest);
        let leftovers: Vec<_> = std::fs::read_dir(&d)
            .unwrap_or_else(|_| std::fs::read_dir(".").expect("cwd"))
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "the temporary file is renamed away");
    }

    #[cfg(unix)]
    #[test]
    fn a_planted_sidecar_symlink_is_refused() {
        let manifest = whisper_tiny();
        let d = dir("sidecar-symlink");
        let outside = d.join("outside.json");
        std::fs::write(&outside, b"{}").unwrap_or(());
        std::os::unix::fs::symlink(&outside, d.join(SIDECAR)).unwrap_or(());
        assert!(write_sidecar(&d, &manifest).is_err(), "the sidecar is not written through a symlink");
        assert_eq!(std::fs::read(&outside).unwrap_or_default(), b"{}", "the link target is untouched");
    }
}
