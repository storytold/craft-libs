#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

//! The envelope of VectorCraft's `.vectorcraft` file, shared by VectorCraft and by the apps that
//! read its files. The document is a type parameter: VectorCraft supplies its own document type,
//! and a reader that needs only the envelope uses [`serde::de::IgnoredAny`].
//!
//! A `.vectorcraft` file is UTF-8 JSON, gzip-compressed when saved with Use Compression:
//!
//! ```text
//! { "format": "vectorcraft", "version": 4, "generator": "VectorCraft 0.17.0",
//!   "preview": { "mime": "image/png", "data": "<base64>" },
//!   "profiles": { "<profile name>": { "mime": "application/vnd.iccprofile", "blob": [0, 9000] } },
//!   "pdf": { "mime": "application/pdf", "blob": [9000, 52000] },
//!   "document": { ... },
//!   "images": { "<key>": { "mime": "image/png", "data": "<base64>" } } }
//! ```
//!
//! Since version 4, entries of [`INLINE_MAX`] bytes or more follow the JSON after [`BLOB_MAGIC`],
//! and such an entry gives `"blob": [offset, length]` (from the end of the marker) instead of
//! `data`. In a compressed file, the marker and the data after it are inside the gzip stream.
//! Files written under the former name say `"format": "drawcraft"`.
//!
//! - [`File`], [`Head`] and [`Image`] are the members, in the order VectorCraft writes them.
//! - [`split_tail`] separates a file's JSON from the data after it; [`Image::bytes`] reads an
//!   entry from either place; [`Tail`] places entries when writing.
//! - [`sniff`] and [`is_compressed`] recognize a file from its first bytes.
//! - [`base64`] encodes and decodes the `data` members.
//!
//! The crate takes and returns bytes. It does not open files or decompress past the first bytes
//! [`sniff`] reads: a reader of a compressed file decompresses it with its own limits first.

pub mod base64;

use std::collections::BTreeMap;
use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

/// The newest version: data after the JSON. Versions 1 to 3 still load.
pub const VERSION: u32 = 4;
/// The oldest version VectorCraft writes.
pub const MIN_VERSION: u32 = 1;
/// The first version whose readers open compressed files.
pub const COMPRESSED_SINCE: u32 = 3;
/// The first version that keeps entries after the JSON.
pub const TAIL_SINCE: u32 = 4;
/// Entries this large or larger go after the JSON (from [`TAIL_SINCE`]), not into it as base64.
pub const INLINE_MAX: usize = 8 << 10;
/// What separates the JSON from the data after it.
pub const BLOB_MAGIC: &[u8] = b"\n\0VCBLOBS\0";
/// The most pixels on the longer side of the embedded `preview`.
pub const PREVIEW_MAX: u32 = 256;
/// The file extension, and the `format` name since version 3.
pub const EXTENSION: &str = "vectorcraft";
/// The extension and `format` name from before the rename (DrawCraft), still read; VectorCraft
/// writes it for versions 1 and 2.
pub const LEGACY_EXTENSION: &str = "drawcraft";
/// The MIME type of embedded ICC profiles.
pub const ICC_MIME: &str = "application/vnd.iccprofile";

/// Why a file's envelope is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("not a VectorCraft file: format is `{0}`")]
    Format(String),
    #[error("file version {0} is newer than this reader supports ({VERSION})")]
    TooNew(u32),
}

/// The top-level object of a file, with the document as `D`. Serialized with `serde_json`, it
/// gives the bytes VectorCraft writes, compact or pretty: the members in this order, `generator`,
/// `preview`, `profiles` and `pdf` left out when absent, `images` always written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct File<D> {
    /// [`EXTENSION`], or [`LEGACY_EXTENSION`] in versions 1 and 2.
    pub format: String,
    pub version: u32,
    /// `"VectorCraft <app version>"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
    /// A PNG of the first artboard, at most [`PREVIEW_MAX`] pixels on its longer side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<Image>,
    /// ICC files by profile name (Embed ICC Profiles).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, Image>,
    /// A PDF of every artboard (Create PDF-Compatible File).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pdf: Option<Image>,
    pub document: D,
    /// The images the document uses, by key.
    #[serde(default)]
    pub images: BTreeMap<String, Image>,
}

impl<D> File<D> {
    /// A file of the current [`VERSION`] holding `document`, with no other members.
    pub fn new(document: D) -> Self {
        Self {
            format: EXTENSION.into(),
            version: VERSION,
            generator: None,
            preview: None,
            profiles: BTreeMap::new(),
            pdf: None,
            document,
            images: BTreeMap::new(),
        }
    }

    /// Is `format` a VectorCraft name, and `version` one this crate reads?
    pub fn check(&self) -> Result<(), Error> {
        check(&self.format, self.version)
    }

    /// Written under the former name (`"format": "drawcraft"`).
    pub fn is_legacy(&self) -> bool {
        self.format == LEGACY_EXTENSION
    }
}

/// [`File::check`] for a `format` and `version` read on their own.
pub fn check(format: &str, version: u32) -> Result<(), Error> {
    if format != EXTENSION && format != LEGACY_EXTENSION {
        return Err(Error::Format(format.into()));
    }
    if version > VERSION {
        return Err(Error::TooNew(version));
    }
    Ok(())
}

/// The members of a file besides its document, which is skipped without being decoded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Head {
    #[serde(default)]
    pub preview: Option<Image>,
    #[serde(default)]
    pub pdf: Option<Image>,
}

/// An entry holding bytes: an image, an ICC profile, the preview or the PDF.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    pub mime: String,
    /// The bytes as base64, unless they come after the JSON (`blob`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub data: String,
    /// Where the bytes are in the data after the JSON: `[offset, length]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<[u64; 2]>,
    /// `data` is a linked image's low-resolution preview, not the linked file's bytes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub proxy: bool,
}

impl Image {
    /// An entry with `bytes` in the JSON, as base64.
    pub fn inline(mime: &str, bytes: &[u8]) -> Self {
        Self { mime: mime.into(), data: base64::encode(bytes), blob: None, proxy: false }
    }

    /// The number of bytes the entry states it holds, without decoding it: the `blob` length, or
    /// the length `data` decodes to.
    pub fn len(&self) -> u64 {
        match self.blob {
            Some([_, len]) => len,
            None => base64::decoded_len(&self.data),
        }
    }

    /// Whether [`Image::len`] is 0.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The entry's bytes: `data` decoded, or the range `blob` names in `tail` (the data after the
    /// JSON, from [`split_tail`]). `None` when `data` is not valid base64 or `tail` ends before the
    /// range does.
    pub fn bytes(&self, tail: &[u8]) -> Option<Vec<u8>> {
        match self.blob {
            Some(at) => entry(tail, at).map(<[u8]>::to_vec),
            None => base64::decode(&self.data),
        }
    }
}

/// The bytes `[offset, length]` names in `tail`; `None` when `tail` ends before them.
pub fn entry(tail: &[u8], [offset, len]: [u64; 2]) -> Option<&[u8]> {
    let start = usize::try_from(offset).ok()?;
    let end = start.checked_add(usize::try_from(len).ok()?)?;
    tail.get(start..end)
}

/// Is this gzip data (a file saved with Use Compression)?
pub fn is_compressed(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x1f, 0x8b])
}

/// Does `head`, the first bytes of a file (compressed or not), start a `.vectorcraft` file? It
/// must hold a JSON object whose top-level `format` member is `"vectorcraft"` or `"drawcraft"`
/// within the first 256 bytes after decompression. VectorCraft writes `format` first.
pub fn sniff(head: &[u8]) -> bool {
    const HEAD: usize = 256;
    let mut unpacked = Vec::new();
    let head = if is_compressed(head) {
        // A head cut short still decompresses its start.
        let _ = flate2::read::GzDecoder::new(head).take(HEAD as u64).read_to_end(&mut unpacked);
        unpacked.as_slice()
    } else {
        head.get(..HEAD).unwrap_or(head)
    };
    format_member(head).is_some_and(|f| f == EXTENSION.as_bytes() || f == LEGACY_EXTENSION.as_bytes())
}

/// The string value of the top-level `format` member of the object `text` starts, as written
/// (escapes are not decoded). `None` when `text` ends before it, or the value is not a string.
fn format_member(text: &[u8]) -> Option<&[u8]> {
    let mut s = Scan { text, at: 0 };
    s.space();
    s.expect(b'{')?;
    loop {
        s.space();
        let key = s.string()?;
        s.space();
        s.expect(b':')?;
        s.space();
        if key == b"format" {
            return s.string();
        }
        s.skip_value()?;
        s.space();
        s.expect(b',')?;
    }
}

/// A position in the first bytes of a JSON text.
struct Scan<'a> {
    text: &'a [u8],
    at: usize,
}

impl<'a> Scan<'a> {
    fn space(&mut self) {
        while self.text.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn expect(&mut self, b: u8) -> Option<()> {
        (*self.text.get(self.at)? == b).then(|| self.at += 1)
    }

    /// The contents of the string that starts here, between its quotes.
    fn string(&mut self) -> Option<&'a [u8]> {
        self.expect(b'"')?;
        let start = self.at;
        loop {
            match *self.text.get(self.at)? {
                b'"' => {
                    self.at += 1;
                    return self.text.get(start..self.at - 1);
                }
                b'\\' => self.at += 2,
                _ => self.at += 1,
            }
        }
    }

    /// Move past the value that starts here: a string, an object or array with everything in it,
    /// or a number or literal up to the `,` or `}` after it.
    fn skip_value(&mut self) -> Option<()> {
        let mut depth = 0usize;
        loop {
            match *self.text.get(self.at)? {
                b'"' => {
                    self.string()?;
                    if depth == 0 {
                        return Some(());
                    }
                }
                b'{' | b'[' => {
                    depth += 1;
                    self.at += 1;
                }
                b'}' | b']' if depth == 0 => return Some(()),
                b'}' | b']' => {
                    depth -= 1;
                    self.at += 1;
                    if depth == 0 {
                        return Some(());
                    }
                }
                b',' if depth == 0 => return Some(()),
                _ => self.at += 1,
            }
        }
    }
}

/// The JSON of an uncompressed (or decompressed) file, without the data after it.
pub fn json_part(file: &[u8]) -> &[u8] {
    split_tail(file).0
}

/// A file's JSON and the data after it, split at [`BLOB_MAGIC`] where it follows the first JSON
/// value. Without the marker there, all of `file` is the JSON and the data after it is empty.
pub fn split_tail(file: &[u8]) -> (&[u8], &[u8]) {
    // JSON holds no raw NUL, so a marker right after the JSON starts just before the first NUL.
    let Some(nul) = file.iter().position(|&b| b == 0) else {
        return (file, &[]);
    };
    if let Some(at) = nul.checked_sub(1)
        && file.get(at..).is_some_and(|r| r.starts_with(BLOB_MAGIC))
        && at.checked_sub(1).and_then(|k| file.get(k)).is_some_and(|b| matches!(b, b'}' | b']' | b'"'))
    {
        return (file.get(..at).unwrap_or(file), file.get(at + BLOB_MAGIC.len()..).unwrap_or_default());
    }
    let mut values = serde_json::Deserializer::from_slice(file).into_iter::<serde::de::IgnoredAny>();
    if values.next().is_some_and(|v| v.is_ok()) {
        let end = values.byte_offset();
        if let Some(rest) = file.get(end..)
            && rest.starts_with(BLOB_MAGIC)
        {
            return (file.get(..end).unwrap_or(file), rest.get(BLOB_MAGIC.len()..).unwrap_or_default());
        }
    }
    (file, &[])
}

/// One entry placed after the JSON by [`Tail::image`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part<'a> {
    pub mime: String,
    pub bytes: &'a [u8],
}

/// The data after the JSON of a file being written. Entries go after the JSON, each at the
/// offset where the one before ends, when the version keeps them there and they are at least
/// [`INLINE_MAX`] bytes; other entries go into the JSON as base64.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tail<'a> {
    parts: Vec<Part<'a>>,
    len: u64,
    after: bool,
}

impl<'a> Tail<'a> {
    /// An empty tail for a file of `version`.
    pub fn new(version: u32) -> Self {
        Self { parts: vec![], len: 0, after: version >= TAIL_SINCE }
    }

    /// The entry for `bytes`, placed after the JSON or inline.
    pub fn image(&mut self, mime: &str, bytes: &'a [u8], proxy: bool) -> Image {
        if self.after && bytes.len() >= INLINE_MAX {
            let len = bytes.len() as u64;
            let at = self.len;
            self.len = self.len.saturating_add(len);
            self.parts.push(Part { mime: mime.into(), bytes });
            return Image { mime: mime.into(), data: String::new(), blob: Some([at, len]), proxy };
        }
        Image { proxy, ..Image::inline(mime, bytes) }
    }

    /// The entries placed after the JSON, in order.
    pub fn parts(&self) -> &[Part<'a>] {
        &self.parts
    }

    /// The bytes of the entries placed after the JSON, together.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// No entry is placed after the JSON.
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// Write what follows the JSON: [`BLOB_MAGIC`] and the entries, or nothing when there are none.
    pub fn write_to(&self, mut w: impl Write) -> io::Result<()> {
        if self.parts.is_empty() {
            return Ok(());
        }
        w.write_all(BLOB_MAGIC)?;
        self.parts.iter().try_for_each(|p| w.write_all(p.bytes))
    }
}
