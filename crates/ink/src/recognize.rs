//! The handwriting-recognition seam: what a recogniser is, a fixed-answer double for tests, and
//! the model descriptor.
//!
//! The runtime is deliberately not here. A host installs a [`Recognizer`] implementation (an
//! inference crate of its own) and calls [`Recognizer::recognize`] with a [`LineImage`] it
//! rendered from a stroke; this crate owns the contract, the error vocabulary and the model's
//! provenance, so the engine and the UI can depend on it without a neural-network runtime.
//!
//! Model weights never ship with this crate. [`ModelDescriptor`] lists the files, their exact
//! sizes and SHA-256 hashes, and [`ModelDescriptor::validate`] checks the descriptor is coherent;
//! hashing and downloading belong to the runtime crate and the application.

/// The square input a small handwritten-text encoder expects.
pub const IMAGE_SIZE: usize = 384;
/// Largest width or height of a [`LineImage`].
pub const MAX_SIDE: u32 = 4096;
/// Most pixels a [`LineImage`] may hold (4 M, 4 MiB of grey data).
pub const MAX_PIXELS: usize = 4 * 1024 * 1024;
/// Longest model file path accepted.
pub const MAX_PATH: usize = 512;
/// Longest text a recogniser may answer with free of charge.
pub const MAX_TEXT: usize = 4096;

/// A grey line image: paper-white background, dark ink. The host draws it; this crate only
/// carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineImage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl LineImage {
    /// An image of `width` × `height` grey pixels, row-major. `None` when a side is zero or over
    /// [`MAX_SIDE`], when the pixel count exceeds [`MAX_PIXELS`], or when `pixels` does not hold
    /// exactly one byte per pixel.
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Option<LineImage> {
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return None;
        }
        let count = usize::try_from(width).ok()?.checked_mul(usize::try_from(height).ok()?)?;
        if count > MAX_PIXELS || pixels.len() != count {
            return None;
        }
        Some(LineImage { width, height, pixels })
    }

    /// The image's width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The image's height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The grey pixels, row-major, one byte per pixel.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

/// Where inference runs.
///
/// CPU is the only backend every platform has; the other variants are the seams an optional
/// accelerator crate would fill, never a requirement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Accelerator {
    /// Pure-Rust CPU inference.
    #[default]
    Cpu,
    /// Intel OpenVINO (NPU/GPU/CPU).
    OpenVino,
    /// Apple Neural Engine / Metal.
    Apple,
}

impl Accelerator {
    /// The wire name commands use.
    pub fn as_str(self) -> &'static str {
        match self {
            Accelerator::Cpu => "cpu",
            Accelerator::OpenVino => "openvino",
            Accelerator::Apple => "apple",
        }
    }

    /// A display name for a UI and for messages.
    pub fn label(self) -> &'static str {
        match self {
            Accelerator::Cpu => "CPU",
            Accelerator::OpenVino => "OpenVINO NPU/GPU",
            Accelerator::Apple => "Apple Neural Engine",
        }
    }

    /// Parses a wire name (`"cpu"`, `"openvino"`, `"apple"`), case-insensitively with common
    /// aliases.
    pub fn parse(name: &str) -> Option<Accelerator> {
        match name.trim().to_ascii_lowercase().replace(['-', '_', ' '], "").as_str() {
            "cpu" => Some(Accelerator::Cpu),
            "openvino" | "npu" | "intelnpu" => Some(Accelerator::OpenVino),
            "apple" | "ane" | "coreml" | "metal" => Some(Accelerator::Apple),
            _ => None,
        }
    }
}

/// Everything that can go wrong before or during recognition. Errors are values, never panics:
/// model files are untrusted input.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The stroke held fewer than one usable point, or was too large to render.
    #[error("{0}")]
    Ink(String),
    /// No recogniser is installed (the model is absent, incomplete or was refused).
    #[error("no handwriting model is installed; its files belong in {0}")]
    NotInstalled(String),
    /// A model file is missing or has the wrong size.
    #[error("the handwriting model is incomplete: {0}")]
    Incomplete(String),
    /// A model file's SHA-256 does not match the descriptor.
    #[error("{0}")]
    Corrupt(String),
    /// The recogniser ran but could not produce text.
    #[error("handwriting recognition failed: {0}")]
    Recognize(String),
    /// A model directory could not be read or written.
    #[error("{0}")]
    Io(String),
}

impl Error {
    /// A one-line, actionable message for a model panel and for command errors.
    pub fn hint(&self) -> String {
        match self {
            Error::NotInstalled(dir) => format!("Download the handwriting model into {dir}."),
            Error::Incomplete(_) | Error::Corrupt(_) => "Open the handwriting model panel and download the model again.".to_string(),
            Error::Ink(message) | Error::Recognize(message) | Error::Io(message) => message.clone(),
        }
    }
}

/// Handwriting recognition a host can install.
///
/// Implementations must be usable from any thread and must not panic on any input: a runtime's
/// failure is [`Error::Recognize`].
pub trait Recognizer: Send + Sync {
    /// Recognises one line of handwriting. The image is paper-white with dark ink.
    fn recognize(&self, image: &LineImage) -> Result<String, Error>;

    /// The descriptor id the recogniser was loaded from (`"trocr-small-handwritten"`).
    fn model_id(&self) -> &'static str;

    /// Where this recogniser runs.
    fn accelerator(&self) -> Accelerator {
        Accelerator::Cpu
    }

    /// The accelerators this recogniser can use in this build, best first.
    fn accelerators(&self) -> &'static [Accelerator] {
        &[Accelerator::Cpu]
    }
}

/// A recogniser that answers with a fixed string, for tests: no model file, no network, no
/// inference.
#[derive(Clone, Debug)]
pub struct FixedRecognizer {
    text: String,
}

impl FixedRecognizer {
    /// The double answering `text`.
    pub fn new(text: impl Into<String>) -> FixedRecognizer {
        FixedRecognizer { text: text.into() }
    }
}

impl Recognizer for FixedRecognizer {
    fn recognize(&self, _image: &LineImage) -> Result<String, Error> {
        if self.text.len() > MAX_TEXT {
            return Err(Error::Recognize("the answer is implausibly long".into()));
        }
        Ok(self.text.clone())
    }

    fn model_id(&self) -> &'static str {
        TROCR_SMALL_HANDWRITTEN.id
    }
}

/// One file of a model, relative to the model's directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelFile {
    /// Path inside the model directory, e.g. `onnx/encoder_model.onnx`.
    pub path: &'static str,
    /// Exact size in bytes.
    pub bytes: u64,
    /// Lowercase hex SHA-256 of the file.
    pub sha256: &'static str,
    /// Where to download it from (HTTPS).
    pub url: &'static str,
}

/// A recogniser model's provenance: what it is, where it comes from and which files it is made
/// of. Hashes are data here; verifying a file is the runtime crate's job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelDescriptor {
    /// Directory name under a model root, also the id commands take.
    pub id: &'static str,
    /// Human-readable name for the UI.
    pub name: &'static str,
    /// The upstream revision the hashes were taken from.
    pub revision: &'static str,
    /// The licence the model card states (not the runtime's).
    pub licence: &'static str,
    /// Where the model comes from, as shown in the download prompt.
    pub source: &'static str,
    /// The runtime and version the descriptor's hashes belong to.
    pub runtime: &'static str,
    /// The accelerators the model can run on, best first. CPU is always there.
    pub accelerators: &'static [Accelerator],
    /// The model's files.
    pub files: &'static [ModelFile],
}

/// TrOCR small (handwritten), fp32 ONNX exported by Xenova: a small line-recognition model used
/// as the reference descriptor.
pub const TROCR_SMALL_HANDWRITTEN: ModelDescriptor = ModelDescriptor {
    id: "trocr-small-handwritten",
    name: "TrOCR small (handwritten)",
    revision: "02bea33ca09a70e2e2d91140a7e6e1a729606a5b",
    licence: "MIT",
    source: "https://huggingface.co/Xenova/trocr-small-handwritten",
    runtime: "rten 0.27 (pure Rust, CPU)",
    accelerators: &[Accelerator::Cpu],
    files: &[
        ModelFile {
            path: "onnx/encoder_model.onnx",
            bytes: 87_496_990,
            sha256: "f6a2c3ac23a426c85f8557f2763bf39cce7eeacf3004563762e023050ddc1fd9",
            url: "https://huggingface.co/Xenova/trocr-small-handwritten/resolve/02bea33ca09a70e2e2d91140a7e6e1a729606a5b/onnx/encoder_model.onnx",
        },
        ModelFile {
            path: "onnx/decoder_model.onnx",
            bytes: 158_814_912,
            sha256: "4cf40055fbca767b2fd34f1de4f86be41d8dac670028bb6f65a3d35ebe0b0e5f",
            url: "https://huggingface.co/Xenova/trocr-small-handwritten/resolve/02bea33ca09a70e2e2d91140a7e6e1a729606a5b/onnx/decoder_model.onnx",
        },
        ModelFile {
            path: "tokenizer.json",
            bytes: 4_494_727,
            sha256: "68bcb5468c854362a615f3d2ff6a5e4091a85f4c8198993ed9a30afe0b143737",
            url: "https://huggingface.co/Xenova/trocr-small-handwritten/resolve/02bea33ca09a70e2e2d91140a7e6e1a729606a5b/tokenizer.json",
        },
    ],
};

impl ModelDescriptor {
    /// Total download size in bytes.
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().fold(0u64, |total, file| total.saturating_add(file.bytes))
    }

    /// The file with this name (`encoder_model.onnx`, also accepted with its path).
    pub fn file_named(&self, name: &str) -> Option<&ModelFile> {
        self.files.iter().find(|file| file.path == name || file.path.rsplit('/').next() == Some(name))
    }

    /// Refuses a descriptor that could escape its model directory or whose hashes make no sense.
    /// The shipped descriptor passes; this guards a future edit one step away from a traversal.
    pub fn validate(&self) -> Result<(), Error> {
        if self.files.is_empty() {
            return Err(Error::Corrupt(format!("{} lists no files", self.id)));
        }
        if !self.accelerators.contains(&Accelerator::Cpu) {
            return Err(Error::Corrupt(format!("{} must run on CPU: that is the backend every platform has", self.id)));
        }
        for file in self.files {
            if !is_safe_relative_path(file.path) {
                return Err(Error::Corrupt(format!("{} has an unsafe file path {:?}", self.id, file.path)));
            }
            if file.bytes == 0 {
                return Err(Error::Corrupt(format!("{} lists {} as empty", self.id, file.path)));
            }
            if !is_sha256_hex(file.sha256) {
                return Err(Error::Corrupt(format!("{} has no valid SHA-256 for {}", self.id, file.path)));
            }
            if !file.url.starts_with("https://") {
                return Err(Error::Corrupt(format!("{} downloads {} over something other than HTTPS", self.id, file.path)));
            }
        }
        Ok(())
    }
}

/// Whether `path` is a safe relative path inside a model directory: no root, no parent
/// component, no drive or UNC prefix, no backslash. A descriptor is data, so a path that escapes
/// its directory must be refused before anything opens it.
pub fn is_safe_relative_path(path: &str) -> bool {
    if path.is_empty() || path.len() > MAX_PATH || path.contains('\\') || path.contains('\0') || path.contains(':') {
        return false;
    }
    if path.starts_with('/') || std::path::Path::new(path).is_absolute() {
        return false;
    }
    let mut parts = 0usize;
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return false;
        }
        parts += 1;
    }
    parts > 0 && std::path::Path::new(path).components().all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// Whether `text` is 64 lowercase hexadecimal digits, the form a descriptor lists.
pub fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_are_bounded() {
        assert!(LineImage::new(2, 2, vec![0; 4]).is_some());
        assert!(LineImage::new(0, 2, vec![0; 4]).is_none());
        assert!(LineImage::new(2, 2, vec![0; 3]).is_none());
        assert!(LineImage::new(MAX_SIDE + 1, 1, vec![0; 4]).is_none());
        assert!(LineImage::new(4096, 1024, vec![0; 4096 * 1024]).is_some());
        let image = LineImage::new(2, 2, vec![7; 4]).expect("a small image");
        assert_eq!((image.width(), image.height(), image.pixels().len()), (2, 2, 4));
    }

    #[test]
    fn the_fixed_recogniser_needs_no_model() {
        let image = LineImage::new(1, 1, vec![255]).expect("an image");
        let recognizer = FixedRecognizer::new("hello");
        assert_eq!(recognizer.recognize(&image), Ok("hello".to_string()));
        assert_eq!(recognizer.model_id(), TROCR_SMALL_HANDWRITTEN.id);
        assert_eq!(recognizer.accelerator(), Accelerator::Cpu);
        assert_eq!(recognizer.accelerators(), &[Accelerator::Cpu]);
    }

    #[test]
    fn the_descriptor_is_coherent() {
        assert_eq!(TROCR_SMALL_HANDWRITTEN.validate(), Ok(()));
        assert_eq!(TROCR_SMALL_HANDWRITTEN.total_bytes(), 87_496_990 + 158_814_912 + 4_494_727);
        assert_eq!(TROCR_SMALL_HANDWRITTEN.file_named("tokenizer.json").map(|f| f.path), Some("tokenizer.json"));
        assert_eq!(TROCR_SMALL_HANDWRITTEN.file_named("../secrets"), None);
    }

    #[test]
    fn unsafe_model_paths_are_refused() {
        for bad in ["", "/abs", "../up", "a/../b", "a//b", "C:/x", "a\\b", "./a", "a\0b"] {
            assert!(!is_safe_relative_path(bad), "{bad}");
        }
        assert!(is_safe_relative_path("onnx/encoder_model.onnx"));
        assert!(!is_sha256_hex("ABC"));
        assert!(is_sha256_hex(&"0".repeat(64)));
        assert!(!is_sha256_hex(&"g".repeat(64)));
    }

    #[test]
    fn accelerators_round_trip_their_names() {
        for accelerator in [Accelerator::Cpu, Accelerator::OpenVino, Accelerator::Apple] {
            assert_eq!(Accelerator::parse(accelerator.as_str()), Some(accelerator));
            assert!(!accelerator.label().is_empty());
        }
        assert_eq!(Accelerator::parse("NPU"), Some(Accelerator::OpenVino));
        assert_eq!(Accelerator::parse("gpu"), None);
        assert!(Error::NotInstalled("/models".into()).hint().contains("/models"));
    }
}
