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
//!   [`File::read`] reads all of them, and [`Contents::read`] reads a file's document, images and
//!   profiles and skips the preview and PDF. Both refuse another format and a newer version.
//! - [`split_tail`] separates a file's JSON from the data after it, and [`marker_at`] finds the
//!   marker in a file's first bytes; [`Entries`] reads a file's entries from either place, copying
//!   at most the size of that data in all; [`Tail`] places entries when writing.
//! - [`sniff`] and [`is_compressed`] recognize a file from its first bytes.
//! - [`base64`] encodes and decodes the `data` members.
//!
//! The crate takes and returns bytes. It does not open files or decompress past the first bytes
//! [`sniff`] reads: a reader of a compressed file decompresses it with its own limits first.

pub mod base64;

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::io::{self, Read, Write};
use std::marker::PhantomData;
use std::sync::Arc;

use serde::de::value::SeqAccessDeserializer;
use serde::de::{self, IgnoredAny, MapAccess, SeqAccess, Visitor};
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
/// The most bytes [`Image::bytes`] gives for one entry: VectorCraft's limit on one entry of the
/// data after the JSON of a compressed file (1 GiB).
pub const MAX_ENTRY: u64 = 1 << 30;

/// Why a file's envelope is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("not a VectorCraft file: format is `{0}`")]
    Format(String),
    #[error("file version {0} is newer than this reader supports ({VERSION})")]
    TooNew(u32),
    #[error("the entries name more bytes than the data after the JSON holds")]
    TooBig,
}

/// [`File::read`] or [`Contents::read`] could not read a file's JSON.
///
/// Other crates match it with a wildcard arm:
///
/// ```compile_fail
/// fn name(e: craft_vectorcraft_format::ReadError) -> &'static str {
///     match e {
///         craft_vectorcraft_format::ReadError::Json(_) => "json",
///         craft_vectorcraft_format::ReadError::Envelope(_) => "envelope",
///     }
/// }
/// ```
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReadError {
    /// The JSON is not a file's top-level object, or the document `D` does not read.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// The JSON reads, but the format is another one ([`Error::Format`]) or the version is newer
    /// than [`VERSION`] ([`Error::TooNew`]).
    #[error(transparent)]
    Envelope(#[from] Error),
}

/// The top-level object of a file, with the document as `D`. Serialized with `serde_json`, it
/// gives the bytes VectorCraft writes, compact or pretty: the members in this order, `generator`,
/// `preview`, `profiles` and `pdf` left out when absent, `images` always written. [`File::read`]
/// reads one. A reader that does not use `generator`, `preview` and `pdf` reads [`Contents`]
/// instead.
///
/// Other crates start one with [`File::new`] and set its members:
///
/// ```compile_fail
/// let _ = craft_vectorcraft_format::File {
///     format: "vectorcraft".into(), version: 4, generator: None, preview: None,
///     profiles: Default::default(), pdf: None, document: 0, images: Default::default(),
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct File<D> {
    /// [`EXTENSION`], or [`LEGACY_EXTENSION`] in versions 1 and 2.
    pub format: String,
    pub version: u32,
    /// `"VectorCraft <app version>"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
    /// A PNG of the first artboard, at most [`PREVIEW_MAX`] pixels on its longer side.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<Image>,
    /// ICC files by profile name (Embed ICC Profiles).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, Image>,
    /// A PDF of every artboard (Create PDF-Compatible File).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdf: Option<Image>,
    pub document: D,
    /// The images the document uses, by key.
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

    /// An empty [`Tail`] for this file's `version`. Set `version` before calling it.
    pub fn tail<'a>(&self) -> Tail<'a> {
        Tail::new(self.version)
    }

    /// Reads a file's JSON (from [`split_tail`]), and refuses another format or a version newer
    /// than [`VERSION`] ([`ReadError::Envelope`]), as VectorCraft does after reading it. A file
    /// reads as VectorCraft opens it:
    ///
    /// - A `generator`, `preview` or `pdf` of the wrong shape reads as `None`. One that appears
    ///   more than once reads as its last value, as `serde_json` reads a repeated key into a
    ///   `serde_json::Value`.
    /// - A missing or repeated `format`, `version` or `document`, or a repeated `images` or
    ///   `profiles`, is refused. Other members are skipped.
    /// - A JSON array holds `format`, `version`, `document`, `images` and `profiles` in that
    ///   order; the last two may be left out.
    pub fn read<'de>(json: &'de [u8]) -> Result<Self, ReadError>
    where
        D: Deserialize<'de>,
    {
        read(json, true)
    }
}

/// [`File::read`] and [`Contents::read`]; `keep` is false for [`Contents`], which skips
/// `generator`, `preview` and `pdf` without keeping them.
fn read<'de, D: Deserialize<'de>>(json: &'de [u8], keep: bool) -> Result<File<D>, ReadError> {
    let mut de = serde_json::Deserializer::from_slice(json);
    let file = serde::Deserializer::deserialize_struct(&mut de, "File", MEMBERS, FileVisitor { keep, document: PhantomData })?;
    de.end()?;
    file.check()?;
    Ok(file)
}

/// The members of a file, in the order of the array form.
const MEMBERS: &[&str] = &["format", "version", "document", "images", "profiles"];

/// The array form of a file, read as VectorCraft reads it.
#[derive(Deserialize)]
#[serde(rename = "File")]
struct Members<D> {
    format: String,
    version: u32,
    document: D,
    #[serde(default)]
    images: BTreeMap<String, Image>,
    #[serde(default)]
    profiles: BTreeMap<String, Image>,
}

/// Reads a [`File`] from a JSON object or array.
struct FileVisitor<D> {
    keep: bool,
    document: PhantomData<D>,
}

impl<'de, D: Deserialize<'de>> Visitor<'de> for FileVisitor<D> {
    type Value = File<D>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("struct File")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<File<D>, A::Error> {
        let Members { format, version, document, images, profiles } = Members::deserialize(SeqAccessDeserializer::new(seq))?;
        Ok(File { format, version, generator: None, preview: None, profiles, pdf: None, document, images })
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<File<D>, A::Error> {
        let (mut format, mut version, mut document, mut images, mut profiles) = (None, None, None, None, None);
        let (mut generator, mut preview, mut pdf) = (None, None, None);
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "format" => once(&mut map, &mut format, "format")?,
                "version" => once(&mut map, &mut version, "version")?,
                "document" => once(&mut map, &mut document, "document")?,
                "images" => once(&mut map, &mut images, "images")?,
                "profiles" => once(&mut map, &mut profiles, "profiles")?,
                "generator" if self.keep => generator = map.next_value::<Lenient<String>>()?.0,
                "preview" if self.keep => preview = map.next_value::<Lenient<Image>>()?.0,
                "pdf" if self.keep => pdf = map.next_value::<Lenient<Image>>()?.0,
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(File {
            format: format.ok_or_else(|| de::Error::missing_field("format"))?,
            version: version.ok_or_else(|| de::Error::missing_field("version"))?,
            generator,
            preview,
            profiles: profiles.unwrap_or_default(),
            pdf,
            document: document.ok_or_else(|| de::Error::missing_field("document"))?,
            images: images.unwrap_or_default(),
        })
    }
}

/// Reads the value of member `name` into `slot`, refusing a second one.
fn once<'de, A: MapAccess<'de>, T: Deserialize<'de>>(map: &mut A, slot: &mut Option<T>, name: &'static str) -> Result<(), A::Error> {
    if slot.is_some() {
        return Err(de::Error::duplicate_field(name));
    }
    *slot = Some(map.next_value()?);
    Ok(())
}

/// An optional member read as `None` when its value has the wrong shape.
struct Lenient<T>(Option<T>);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Lenient<T> {
    fn deserialize<De: serde::Deserializer<'de>>(d: De) -> Result<Self, De::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Shape<T> {
            Value(T),
            Other(IgnoredAny),
        }
        Ok(Lenient(match Shape::deserialize(d)? {
            Shape::Value(v) => Some(v),
            Shape::Other(_) => None,
        }))
    }
}

/// [`Lenient`] as a `deserialize_with` function.
fn lenient<'de, De: serde::Deserializer<'de>, T: Deserialize<'de>>(d: De) -> Result<Option<T>, De::Error> {
    Lenient::deserialize(d).map(|l| l.0)
}

/// [`File::check`] for a `format` and `version` read on their own. Every version up to
/// [`VERSION`] passes, 0 included, as VectorCraft opens them; [`MIN_VERSION`] limits only the
/// versions VectorCraft writes.
pub fn check(format: &str, version: u32) -> Result<(), Error> {
    if format != EXTENSION && format != LEGACY_EXTENSION {
        return Err(Error::Format(format.into()));
    }
    if version > VERSION {
        return Err(Error::TooNew(version));
    }
    Ok(())
}

/// The members of a file that a reader of its document needs: [`File`] without `generator`,
/// `preview` and `pdf`, in the order of the array form. Only [`Contents::read`] makes one:
///
/// ```compile_fail
/// let _ = craft_vectorcraft_format::Contents {
///     format: "vectorcraft".into(), version: 4, document: 0,
///     images: Default::default(), profiles: Default::default(),
/// };
/// ```
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Contents<D> {
    pub format: String,
    pub version: u32,
    pub document: D,
    pub images: BTreeMap<String, Image>,
    pub profiles: BTreeMap<String, Image>,
}

impl<D> Contents<D> {
    /// Reads a file's JSON as [`File::read`] does, and skips `generator`, `preview` and `pdf` as
    /// it reads them, so their data is neither buffered nor kept. Any value of those three is
    /// accepted, however many times it appears.
    pub fn read<'de>(json: &'de [u8]) -> Result<Self, ReadError>
    where
        D: Deserialize<'de>,
    {
        let File { format, version, document, images, profiles, .. } = read(json, false)?;
        Ok(Self { format, version, document, images, profiles })
    }

    /// [`File::is_legacy`].
    pub fn is_legacy(&self) -> bool {
        self.format == LEGACY_EXTENSION
    }
}

/// The members of a file besides its document, which is skipped without being decoded. A
/// `preview` or `pdf` of the wrong shape reads as `None`, as in [`File`]; a repeated `preview` or
/// `pdf` is refused.
///
/// ```compile_fail
/// let _ = craft_vectorcraft_format::Head { preview: None, pdf: None };
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct Head {
    #[serde(default, deserialize_with = "lenient")]
    pub preview: Option<Image>,
    #[serde(default, deserialize_with = "lenient")]
    pub pdf: Option<Image>,
}

/// An entry holding bytes: an image, an ICC profile, the preview or the PDF. Other crates make
/// one with [`Image::inline`], [`Tail::image`] or `Image::default()`:
///
/// ```compile_fail
/// let _ = craft_vectorcraft_format::Image { mime: String::new(), data: String::new(), blob: None, proxy: false };
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
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
    /// JSON, from [`split_tail`]). `None` when the entry states more than [`MAX_ENTRY`] bytes,
    /// when `data` is not valid base64, or when `tail` ends before the range does. Each call
    /// copies the range again; read the entries of one file through [`Entries`], which limits the
    /// bytes copied in all.
    pub fn bytes(&self, tail: &[u8]) -> Option<Vec<u8>> {
        self.bytes_at_most(tail, MAX_ENTRY)
    }

    /// [`Image::bytes`] with a limit of `max` bytes instead of [`MAX_ENTRY`]: `None` when
    /// [`Image::len`] is more than `max`, checked before anything is decoded or copied.
    pub fn bytes_at_most(&self, tail: &[u8], max: u64) -> Option<Vec<u8>> {
        if self.len() > max {
            return None;
        }
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

/// Reads the entries of one file, copying at most as many bytes from the data after the JSON as
/// that data holds, in all. A file VectorCraft writes names each of those bytes once, so all of
/// its entries read. Entries that name the same `[offset, length]` share one copy, which counts
/// once. Entries whose different ranges overlap copy the shared bytes once each, and are refused
/// once their copies pass that size, as VectorCraft's reader refuses them. Unlike
/// [`Image::bytes`], `Entries` puts no limit on one entry: VectorCraft opens an uncompressed file
/// whose entry is larger than [`MAX_ENTRY`].
#[derive(Debug, Clone)]
pub struct Entries<'a> {
    tail: &'a [u8],
    copied: u64,
    done: HashMap<[u64; 2], Arc<Vec<u8>>>,
}

impl<'a> Entries<'a> {
    /// A reader of entries over `tail`, the data after the JSON (from [`split_tail`]).
    pub fn new(tail: &'a [u8]) -> Self {
        Self { tail, copied: 0, done: HashMap::new() }
    }

    /// The entry's bytes: `data` decoded (`None` when it is not valid base64), or
    /// [`Entries::blob`] of its range.
    pub fn bytes(&mut self, image: &Image) -> Result<Option<Arc<Vec<u8>>>, Error> {
        match image.blob {
            Some(at) => self.blob(at),
            None => Ok(base64::decode(&image.data).map(Arc::new)),
        }
    }

    /// The bytes `[offset, length]` names in `tail` (`None` when `tail` ends before them). A range
    /// this reader has already read gets the same buffer. [`Error::TooBig`] when the bytes this
    /// reader has copied from `tail`, these included, would be more than `tail` holds; a refused
    /// range is not counted.
    pub fn blob(&mut self, at: [u64; 2]) -> Result<Option<Arc<Vec<u8>>>, Error> {
        if let Some(bytes) = self.done.get(&at) {
            return Ok(Some(Arc::clone(bytes)));
        }
        let Some(bytes) = entry(self.tail, at) else {
            return Ok(None);
        };
        let copied = self.copied.saturating_add(bytes.len() as u64);
        if copied > self.tail.len() as u64 {
            return Err(Error::TooBig);
        }
        self.copied = copied;
        let bytes = Arc::new(bytes.to_vec());
        self.done.insert(at, Arc::clone(&bytes));
        Ok(Some(bytes))
    }

    /// The bytes copied from `tail` so far.
    pub fn copied(&self) -> u64 {
        self.copied
    }
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
    format_member(head).is_some_and(|f| f == EXTENSION || f == LEGACY_EXTENSION)
}

/// The string value of the top-level `format` member of the object `text` starts, with keys and
/// values compared as JSON reads them (escapes decoded). `None` when `text` ends before it, or the
/// value is not a string.
fn format_member(text: &[u8]) -> Option<String> {
    let mut s = Scan { text, at: 0 };
    s.space();
    s.expect(b'{')?;
    loop {
        s.space();
        let key = s.decoded()?;
        s.space();
        s.expect(b':')?;
        s.space();
        if key == "format" {
            return s.decoded();
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

    /// The string that starts here, as JSON reads it.
    fn decoded(&mut self) -> Option<String> {
        let start = self.at;
        self.string()?;
        serde_json::from_slice(self.text.get(start..self.at)?).ok()
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
    if !file.contains(&0) {
        return (file, &[]);
    }
    if let Some(at) = marker_at(file) {
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

/// Where [`BLOB_MAGIC`] starts in `file` when it is where VectorCraft writes it: right after a
/// JSON object, array or string, and holding the first NUL of `file`, which JSON never contains.
/// `file` may be the first bytes of a file, up to the end of the marker. [`split_tail`] splits
/// there first; a reader that unpacks a compressed file as a stream finds the end of its JSON with
/// it.
pub fn marker_at(file: &[u8]) -> Option<usize> {
    let at = file.iter().position(|&b| b == 0)?.checked_sub(1)?;
    (file.get(at..)?.starts_with(BLOB_MAGIC) && matches!(file.get(at.checked_sub(1)?)?, b'}' | b']' | b'"')).then_some(at)
}

/// One entry placed after the JSON by [`Tail::image`].
///
/// ```compile_fail
/// let _ = craft_vectorcraft_format::Part { mime: String::new(), bytes: &[] };
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
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
    /// An empty tail for a file of `version`, which is the file's own `version`; [`File::tail`]
    /// takes it from the file.
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
