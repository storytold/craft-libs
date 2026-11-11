//! File thumbnails: `FileThumbKey`, `ThumbDecoder`, `DecodeRequest`, `DecodedThumb`, `ThumbError`,
//! `ThumbStore` and `StoreKey` without egui; with `ui`, `FileThumbSource` and `FileThumbOptions`,
//! one more `ThumbSource` over the shared `craft_ui::primitives::textures::Thumbnails` cache.
//!
//! ```ignore
//! let files = FileThumbSource::new(Arc::new(MyDecoder), Some(Arc::new(MyStore)));   // app state, one per library
//! let thumbs = Thumbnails::<PhotoId>::in_context(ctx, "library", ThumbnailOptions::new());
//! let source = files.keyed(|id: &PhotoId| catalog.file(*id));                        // per pass: tile key to FileThumbKey
//! let state = thumbs.state(ctx, &source, &id, &request, Priority::Visible);          // then `thumbs.paint(..)` in the tile's picture
//! ```

#[cfg(feature = "ui")]
mod source;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use craft_host::queue::Cancel;

#[cfg(feature = "ui")]
pub use source::{FileThumbOptions, FileThumbSource, KeyedFileThumbs};

/// Which picture of which file: the path and an app-defined variant (a page number, a frame time
/// in ticks, 0 for a plain image).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FileThumbKey {
    /// The file.
    pub path: PathBuf,
    /// The app's variant of it.
    pub variant: u64,
}

impl FileThumbKey {
    /// The whole file (variant 0).
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), variant: 0 }
    }
}

/// A decode job's question: which picture, how big, and the file's version as the job found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeRequest {
    /// The picture.
    pub key: FileThumbKey,
    /// The longest side wanted, in pixels (a multiple of 16, from `ThumbRequest::side`).
    pub side: u32,
    /// The file's length, from the job's stat.
    pub len: u64,
    /// The file's modification time, from the job's stat.
    pub modified: Option<SystemTime>,
}

/// A file's length and modification time, as the decode job's stat found them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStat {
    /// The file's length.
    pub len: u64,
    /// Its modification time, when the platform has one.
    pub modified: Option<SystemTime>,
}

/// A decoded picture: unmultiplied sRGBA rows, longest side at most the request's `side`, the
/// file's aspect kept (no letterbox bars).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedThumb {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width × height × 4` bytes, top row first.
    pub rgba: Vec<u8>,
}

/// Why a file has no thumbnail. Shown as the thumbnail's failure tooltip, so `Failed` carries a
/// message in the user's language.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ThumbError {
    /// The file isn't there.
    Missing,
    /// The decoder doesn't read this kind of file: the tile shows no picture (state `Empty`) and
    /// the host's own icon, not a failure.
    Unsupported,
    /// The decoder failed; the message.
    Failed(String),
    /// The job's `Cancel` was set; nothing is shown or remembered.
    Cancelled,
}

/// The app's decoder (raw, PDF page, video frame, image). Called only in jobs.
pub trait ThumbDecoder: Send + Sync {
    /// The full thumbnail. Checks `cancel` between expensive steps and returns `Cancelled` when
    /// it is set.
    fn decode(&self, request: &DecodeRequest, cancel: &Cancel) -> Result<DecodedThumb, ThumbError>;

    /// The file's length and modification time, asked in the job before `decode`. The default
    /// reads the file system (`Missing` when the file can't be statted); on wasm32 it has no file
    /// system and answers `Unsupported`, so a web app overrides it. Tests give a fake.
    fn stat(&self, path: &Path) -> Result<FileStat, ThumbError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let meta = std::fs::metadata(path).map_err(|_| ThumbError::Missing)?;
            Ok(FileStat { len: meta.len(), modified: meta.modified().ok() })
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = path;
            Err(ThumbError::Unsupported)
        }
    }

    /// An embedded preview to show first (a raw file's JPEG, LightCraft's `ThumbQuick`), when the
    /// file has one that is cheaper than `decode`. The default has none.
    fn quick(&self, _request: &DecodeRequest, _cancel: &Cancel) -> Option<DecodedThumb> {
        None
    }
}

/// Where a stored thumbnail is found: the file, its version and the side.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StoreKey {
    /// The picture.
    pub key: FileThumbKey,
    /// The file's length when the thumbnail was made.
    pub len: u64,
    /// Its modification time then.
    pub modified: Option<SystemTime>,
    /// The longest side.
    pub side: u32,
}

/// The app's on-disk thumbnail store (LightCraft's engine cache); optional. Called only in jobs.
/// No folder store is built in the library (thumbs lead call 24).
pub trait ThumbStore: Send + Sync {
    /// The stored thumbnail for exactly this version and side, if any.
    fn load(&self, key: &StoreKey) -> Option<DecodedThumb>;

    /// Stores a thumbnail `decode` made. Errors are the store's to log; a failed save changes
    /// nothing shown.
    fn save(&self, key: &StoreKey, thumb: &DecodedThumb);
}
