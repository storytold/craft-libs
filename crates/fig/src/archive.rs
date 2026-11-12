//! The zip around `canvas.fig`.

use std::cell::Cell;

use craft_zip::Compression;

use crate::canvas::MAX_CANVAS;
use crate::images::MAX_IMAGE_HASH;
use crate::{Error, corrupt};

/// Largest `meta.json` read.
const MAX_META: u64 = 1 << 20;
/// Largest file [`write_archive`] writes (the canvas and the images together).
const MAX_FILE: u64 = 2 * 1024 * 1024 * 1024;
/// Most entries an archive may declare; checked before the archive is indexed.
pub const MAX_ENTRIES: u64 = 1_000_000;

/// What one open may read from `images/`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Largest image member read.
    pub max_image: u64,
    /// Total decompressed image bytes read, damaged images included.
    pub max_image_total: u64,
}

impl Default for Limits {
    /// 256 MiB an image; 4 GiB in all, 1 GiB on 32-bit targets (to leave address space for the
    /// canvas, the layers and decoded pixels).
    fn default() -> Self {
        #[cfg(not(target_pointer_width = "32"))]
        let max_image_total = 4 * 1024 * 1024 * 1024;
        #[cfg(target_pointer_width = "32")]
        let max_image_total = 1024 * 1024 * 1024;
        Limits { max_image: 256 * 1024 * 1024, max_image_total }
    }
}

/// Whether `bytes` look like a `.fig` file: a zip holding `canvas.fig`, or a bare canvas.
pub fn is_fig(bytes: &[u8]) -> bool {
    bytes.starts_with(b"fig-") || (bytes.starts_with(b"PK") && open_zip(bytes).is_ok_and(|z| z.contains("canvas.fig")))
}

/// The parts of a `.fig` archive (or of a bare canvas, which has no images).
pub struct Archive<'b> {
    /// `canvas.fig`, still chunked and compressed ([`crate::decode_canvas`] reads it). Public so
    /// that a reader can take it out and drop it once decoded: in a large file every stage takes
    /// gigabytes.
    pub canvas: Vec<u8>,
    /// `meta.json`'s `file_name`.
    pub file_name: Option<String>,
    /// `images/<hash>` members (or nothing for a bare canvas).
    zip: Option<craft_zip::Archive<'b>>,
    max_image: u64,
    /// Bytes left for image reads. A reader that caches what it resolved, failures included,
    /// reads or counts each referenced member only once.
    remaining: Cell<u64>,
    skipped: Cell<usize>,
}

impl<'b> Archive<'b> {
    /// Open `bytes` with the default [`Limits`].
    pub fn open(bytes: &'b [u8]) -> Result<Self, Error> {
        Self::open_with(bytes, Limits::default())
    }

    pub fn open_with(bytes: &'b [u8], limits: Limits) -> Result<Self, Error> {
        let (max_image, remaining, skipped) = (limits.max_image, Cell::new(limits.max_image_total), Cell::new(0));
        if bytes.starts_with(b"fig-") {
            return Ok(Archive { canvas: bytes.to_vec(), file_name: None, zip: None, max_image, remaining, skipped });
        }
        let mut z = open_zip(bytes).map_err(|e| corrupt(e.to_string()))?;
        // `zip` keeps one entry of a repeated name; which one the writer meant is not known.
        if z.has_duplicate_names() {
            return Err(corrupt("two entries of the archive share a name"));
        }
        let canvas = read_member(&mut z, "canvas.fig", MAX_CANVAS)?;
        let file_name = read_member(&mut z, "meta.json", MAX_META)
            .ok()
            .and_then(|m| serde_json::from_slice::<serde_json::Value>(&m).ok())
            .and_then(|m| m.get("file_name").and_then(serde_json::Value::as_str).map(str::to_string));
        Ok(Archive { canvas, file_name, zip: Some(z), max_image, remaining, skipped })
    }

    /// Whether the canvas has FigJam's header: the file is a whiteboard (a `.jam`).
    pub fn is_board(&self) -> bool {
        self.canvas.starts_with(crate::JAM_HEADER)
    }

    /// Whether `images/` has a member for image `hash`.
    pub fn has_image(&self, hash: &str) -> bool {
        self.zip.as_ref().is_some_and(|z| z.contains(&format!("images/{hash}")))
    }

    /// What is left of the total image budget ([`Limits::max_image_total`]).
    pub fn image_bytes_left(&self) -> u64 {
        self.remaining.get()
    }

    /// Images left out because the total image budget ([`Limits::max_image_total`]) was spent.
    pub fn images_skipped(&self) -> usize {
        self.skipped.get()
    }

    /// Bytes of image `hash` (hex) from `images/`. `None` when there is no such member, it is
    /// larger than [`Limits::max_image`], damaged, or past what is left of the total budget (then
    /// it counts in [`Self::images_skipped`]).
    pub fn image(&self, hash: &str) -> Option<Vec<u8>> {
        if hash.is_empty() || hash.len() > 2 * MAX_IMAGE_HASH || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        // A clone shares the parsed directory and the bytes; the resolver only has shared access.
        let mut z = self.zip.clone()?;
        let member = format!("images/{hash}");
        let size = z.declared_size(&member)?;
        if size > self.max_image {
            return None;
        }
        let remaining = self.remaining.get();
        if size > remaining {
            self.skipped.set(self.skipped.get() + 1);
            return None;
        }
        // The read stops one byte past the declared size, so a failed read (a bad checksum, a
        // stream that says it is shorter than it is) cost at most that: it keeps the charge.
        let read = z.read(&member, size);
        let used = match &read {
            Ok(Some(data)) => data.len() as u64,
            _ => size,
        };
        self.remaining.set(remaining.saturating_sub(used));
        read.ok().flatten()
    }
}

/// The archive, its declared entry count checked before it is indexed. The image budget is this
/// crate's own ([`Limits`]), so the archive's total is not used.
fn open_zip(bytes: &[u8]) -> Result<craft_zip::Archive<'_>, craft_zip::Error> {
    craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: MAX_ENTRIES, max_total_bytes: u64::MAX })
}

/// Member `name` of `z`, refused when it is (or unpacks to) more than `limit` bytes.
fn read_member(z: &mut craft_zip::Archive<'_>, name: &str, limit: u64) -> Result<Vec<u8>, Error> {
    match z.read(name, limit) {
        Ok(Some(data)) => Ok(data),
        Ok(None) => Err(corrupt(format!("{name}: not in the archive"))),
        Err(craft_zip::Error::EntryTooLarge { .. }) => Err(corrupt(format!("{name} is too large"))),
        Err(e) => Err(corrupt(e.to_string())),
    }
}

/// What [`write_archive`] puts in a `.fig` file.
#[derive(Clone, Copy, Debug)]
pub struct Package<'a> {
    /// `meta.json`'s `file_name`.
    pub file_name: &'a str,
    /// `meta.json`'s `client_meta.background_color`: red, green, blue, alpha, each 0–1.
    pub background: [f64; 4],
    /// `canvas.fig` ([`crate::encode_canvas`]).
    pub canvas: &'a [u8],
    /// `images/<name>` members: the name (the image's hash in hex) and the bytes.
    pub images: &'a [(&'a str, &'a [u8])],
    /// `thumbnail.png`; a PNG's size goes into `client_meta.thumbnail_size`.
    pub thumbnail: Option<&'a [u8]>,
}

/// The `.fig` zip of `package`: `canvas.fig`, `thumbnail.png`, `meta.json`, then the images
/// (stored, as they are compressed already; `meta.json` deflated).
pub fn write_archive(package: &Package) -> Result<Vec<u8>, Error> {
    let io = |e: &dyn std::fmt::Display| Error::Io(e.to_string());
    let mut z = craft_zip::Writer::new();
    let (stored, deflated) = (Compression::Stored, Compression::Level(3));
    {
        let mut put =
            |name: &str, compression: Compression, bytes: &[u8]| -> Result<(), Error> { z.add(name, bytes, compression).map_err(|e| io(&e)) };
        put("canvas.fig", stored, package.canvas)?;
        if let Some(t) = package.thumbnail {
            put("thumbnail.png", stored, t)?;
        }
        let [r, g, b, a] = package.background;
        let mut client = serde_json::json!({"background_color": {"r": r, "g": g, "b": b, "a": a}});
        // A PNG says its size right after its signature and header chunk name.
        let be = |at: usize| package.thumbnail.and_then(|t| t.get(at..at + 4)).and_then(|b| b.try_into().ok()).map(u32::from_be_bytes);
        if let (Some(w), Some(h)) = (be(16), be(20))
            && let Some(client) = client.as_object_mut()
        {
            client.insert("thumbnail_size".into(), serde_json::json!({"width": w, "height": h}));
        }
        let meta = serde_json::to_vec(&serde_json::json!({"client_meta": client, "file_name": package.file_name})).map_err(|e| io(&e))?;
        put("meta.json", deflated, &meta)?;
        let mut total = package.canvas.len() as u64;
        for (name, bytes) in package.images {
            total = total.saturating_add(bytes.len() as u64);
            if total > MAX_FILE {
                return Err(Error::TooLarge);
            }
            if name.is_empty() || !name.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(Error::Io("an image is named by its hash in hex".into()));
            }
            put(&format!("images/{name}"), stored, bytes)?;
        }
    }
    z.finish().map_err(|e| io(&e))
}
