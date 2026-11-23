//! The native `.deckcraft` package: a ZIP holding a stored `mimetype`, `presentation.json` (the
//! manifest: format name, version, generator and the presentation, minus media bytes), and
//! `media/<id>` and `chart-parts/<index>` binary members. Readers ignore unknown fields, so newer
//! files open in older builds with what those builds understand.
//!
//! DeckCraft owns the presentation model and its mapping: which media and chart parts become
//! members, and how a loaded deck is repaired. This crate owns the bytes: the package identity,
//! the manifest envelope, the NaN-safe JSON writer, the reader that opens files with NaN saved as
//! `null`, member names and compression, and the bounded member reads and their budgets.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod finite;
mod manifest;

#[cfg(test)]
mod tests;

use std::io::{Cursor, Read};

pub use finite::Finite;
pub use manifest::parse_manifest;

pub const MIMETYPE: &str = "application/vnd.storyteller.deckcraft+zip";
/// Files written before the SlideCraft → DeckCraft rename (`.slidecraft`) are still read.
pub const LEGACY_MIMETYPE: &str = concat!("application/vnd.storyteller.", "slide", "craft+zip");
pub const EXTENSION: &str = "deckcraft";
/// The manifest's `format`.
pub const FORMAT: &str = "deckcraft";
/// Format version written into the manifest.
pub const VERSION: u32 = 1;
/// The stored member naming the package type.
pub const MIMETYPE_ENTRY: &str = "mimetype";
/// The manifest member.
pub const MANIFEST_ENTRY: &str = "presentation.json";
/// Largest entry we will inflate (guards against zip bombs).
pub const MAX_ENTRY: u64 = 2 << 30;
/// All embedded media and chart sidecars together, inflated (storytold/deckcraft#393).
pub const MAX_TOTAL_MEDIA: u64 = 4 << 30;
/// Media items and chart sidecar parts one file may list.
pub const MAX_MEDIA_ITEMS: usize = 100_000;
/// Largest `presentation.json` we will inflate. The manifest holds no media bytes, so it is far
/// smaller than a media entry may be; inflating and parsing one is what dominates memory.
pub const MAX_MANIFEST: u64 = 256 << 20;

const MEDIA_PREFIX: &str = "media/";
const CHART_PARTS_PREFIX: &str = "chart-parts/";

/// What one load may inflate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    /// Bytes all media and chart members may inflate to together.
    pub total: u64,
    /// Media items and chart parts the manifest may list.
    pub items: usize,
    /// Bytes `presentation.json` may inflate to.
    pub manifest: u64,
}

impl Default for Budget {
    fn default() -> Self {
        Budget { total: MAX_TOTAL_MEDIA, items: MAX_MEDIA_ITEMS, manifest: MAX_MANIFEST }
    }
}

impl Budget {
    /// A media budget with the default manifest limit.
    pub const fn media(total: u64, items: usize) -> Self {
        Budget { total, items, manifest: MAX_MANIFEST }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("not a DeckCraft file: {0}")]
    NotOurs(String),
    #[error("damaged file: {0}")]
    Damaged(String),
    #[error("write failed: {0}")]
    Write(String),
}

pub type Result<T> = std::result::Result<T, FormatError>;

/// `presentation.json`: the envelope around the app's presentation.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Manifest<T> {
    pub format: String,
    pub version: u32,
    pub generator: String,
    pub presentation: T,
}

/// The member holding media item `id`'s bytes.
pub fn media_entry(id: u32) -> String {
    format!("{MEDIA_PREFIX}{id}")
}

/// The member holding the `index`th chart part a save separated out.
pub fn chart_part_entry(index: usize) -> String {
    format!("{CHART_PARTS_PREFIX}{index}")
}

/// Whether `entry` is a name [`chart_part_entry`] gives, exactly: no other member, no path
/// tricks, no leading zeros.
pub fn is_chart_part_entry(entry: &str) -> bool {
    entry.strip_prefix(CHART_PARTS_PREFIX).and_then(|s| s.parse::<usize>().ok()).is_some_and(|n| entry == chart_part_entry(n))
}

/// Whether a member of this content type is stored rather than deflated: it is compressed already.
pub fn is_compressed(content_type: &str) -> bool {
    content_type.starts_with("image/jpeg")
        || content_type.starts_with("image/png")
        || content_type.starts_with("video/")
        || content_type.starts_with("audio/mpeg")
        || content_type.starts_with("audio/mp4")
        || content_type.contains("zip")
}

/// Does this look like a `.deckcraft` file?
pub fn sniff(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK") && [MIMETYPE, LEGACY_MIMETYPE].iter().any(|m| bytes.windows(m.len()).take(200).any(|w| w == m.as_bytes()))
}

/// A binary member of a package being written.
#[derive(Clone, Copy, Debug)]
pub struct Member<'a> {
    /// Its name, from [`media_entry`] or [`chart_part_entry`].
    pub name: &'a str,
    /// Its content type, which decides whether it is stored or deflated.
    pub content_type: &'a str,
    pub bytes: &'a [u8],
}

/// The package: `mimetype` (stored), the manifest naming `generator` around `presentation`
/// (deflated, with non-finite numbers written as 0), then `members` in order.
pub fn write<T: serde::Serialize>(presentation: &T, generator: &str, members: &[Member<'_>]) -> Result<Vec<u8>> {
    use craft_zip::Compression::{Deflated, Stored};
    let w = |e: craft_zip::Error| FormatError::Write(e.to_string());
    let mut z = craft_zip::Writer::new();
    z.add(MIMETYPE_ENTRY, MIMETYPE.as_bytes(), Stored).map_err(w)?;
    let m = Manifest { format: FORMAT.to_string(), version: VERSION, generator: generator.to_string(), presentation };
    let json = serde_json::to_vec(&Finite(&m)).map_err(|e| FormatError::Write(e.to_string()))?;
    z.add(MANIFEST_ENTRY, &json, Deflated).map_err(w)?;
    for member in members {
        let compression = if is_compressed(member.content_type) { Stored } else { Deflated };
        z.add(member.name, member.bytes, compression).map_err(w)?;
    }
    z.finish().map_err(w)
}

/// A package open for reading. Nothing is inflated until a member is read.
pub struct Package<'a> {
    zip: zip::ZipArchive<Cursor<&'a [u8]>>,
}

impl<'a> Package<'a> {
    /// Index `bytes` as a ZIP.
    pub fn open(bytes: &'a [u8]) -> Result<Self> {
        let zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| FormatError::NotOurs(e.to_string()))?;
        Ok(Package { zip })
    }

    /// Open `bytes` and read its manifest, inflating at most `max_manifest` bytes of it. Missing
    /// `presentation.json` or a different `format` is [`FormatError::NotOurs`].
    pub fn open_manifest<T: serde::de::DeserializeOwned>(bytes: &'a [u8], max_manifest: u64) -> Result<(Manifest<T>, Self)> {
        let mut package = Package::open(bytes)?;
        if !package.contains(MANIFEST_ENTRY) {
            return Err(FormatError::NotOurs(format!("no {MANIFEST_ENTRY}")));
        }
        let json = package.read_max(MANIFEST_ENTRY, max_manifest)?;
        let m: Manifest<T> = parse_manifest(&json)?;
        if m.format != FORMAT {
            return Err(FormatError::NotOurs(m.format));
        }
        Ok((m, package))
    }

    /// Whether the package has a member named `name`.
    pub fn contains(&self, name: &str) -> bool {
        self.zip.index_for_name(name).is_some()
    }

    /// The size `name` declares, inflated; `None` when there is no such readable member.
    pub fn declared_size(&mut self, name: &str) -> Option<u64> {
        self.zip.by_name(name).ok().map(|f| f.size())
    }

    /// Read one member within [`MAX_ENTRY`].
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>> {
        self.read_max(name, MAX_ENTRY)
    }

    /// Read one member, refusing it before inflating if it declares more than `max` bytes, and
    /// stopping at `max` if it inflates further than it declared.
    pub fn read_max(&mut self, name: &str, max: u64) -> Result<Vec<u8>> {
        self.read_counted(name, max, &mut 0)
    }

    /// [`Package::read_max`] within `left` (and [`MAX_ENTRY`]), charging `left` for every byte
    /// inflated, including those inflated before a decode or CRC error, so a member that fails
    /// late still spends the shared budget it used.
    pub fn read_charged(&mut self, name: &str, left: &mut u64) -> Result<Vec<u8>> {
        let mut spent = 0;
        let out = self.read_counted(name, (*left).min(MAX_ENTRY), &mut spent);
        *left = left.saturating_sub(spent);
        out
    }

    /// [`Package::read_max`], adding the bytes inflated to `spent` whether or not the read succeeds.
    fn read_counted(&mut self, name: &str, max: u64, spent: &mut u64) -> Result<Vec<u8>> {
        let f = self.zip.by_name(name).map_err(|e| FormatError::Damaged(format!("{name}: {e}")))?;
        if f.size() > max {
            return Err(FormatError::Damaged(format!("{name} is too large")));
        }
        let mut out = Vec::with_capacity(usize::try_from(f.size().min(64 << 20)).unwrap_or(0));
        let mut f = f.take(max.saturating_add(1));
        let read = f.read_to_end(&mut out);
        *spent = spent.saturating_add(out.len() as u64);
        read.map_err(|e| FormatError::Damaged(format!("{name}: {e}")))?;
        if out.len() as u64 > max {
            return Err(FormatError::Damaged(format!("{name} is too large")));
        }
        Ok(out)
    }
}
