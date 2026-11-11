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
mod container;

pub use container::{Entries, Part, Tail, entry, is_compressed, json_part, marker_at, sniff, split_tail};

use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

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
    /// The `format` member is not [`EXTENSION`] or [`LEGACY_EXTENSION`]; it holds the value found.
    #[error("not a VectorCraft file: format is `{0}`")]
    Format(String),
    /// The file's version is newer than [`VERSION`]; it holds the file's version.
    #[error("file version {0} is newer than this reader supports ({VERSION})")]
    TooNew(u32),
    /// The entries name more bytes than the data after the JSON holds.
    #[error("the entries name more bytes than the data after the JSON holds")]
    TooBig,
}

/// [`File::read`] or [`Contents::read`] could not read a file's JSON.
///
/// Other crates match it with a wildcard arm.
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
/// Other crates start one with [`File::new`] and set its members.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct File<D> {
    /// [`EXTENSION`], or [`LEGACY_EXTENSION`] in versions 1 and 2.
    pub format: String,
    /// The file format version, at most [`VERSION`] for a file this reader accepts.
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
    /// The document, in the app's own type.
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
        read(json, true, false).map(|(file, _)| file)
    }
}

/// [`File::read`] and [`Contents::read`]; `keep` is false for [`Contents`], which skips
/// `generator`, `preview` and `pdf` without keeping them.
fn read<'de, D: Deserialize<'de>>(json: &'de [u8], keep: bool, read_pdf: bool) -> Result<(File<D>, bool), ReadError> {
    let mut de = serde_json::Deserializer::from_slice(json);
    let mut pdf = PdfPreference::default();
    let file = serde::Deserializer::deserialize_struct(
        &mut de,
        "File",
        MEMBERS,
        FileVisitor { keep, pdf: read_pdf.then_some(&mut pdf), document: PhantomData },
    )?;
    de.end()?;
    file.check()?;
    Ok((file, pdf.preference.unwrap_or(pdf.present)))
}

#[derive(Default)]
struct PdfPreference {
    preference: Option<bool>,
    present: bool,
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
struct FileVisitor<'a, D> {
    keep: bool,
    pdf: Option<&'a mut PdfPreference>,
    document: PhantomData<D>,
}

impl<'de, D: Deserialize<'de>> Visitor<'de> for FileVisitor<'_, D> {
    type Value = File<D>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("struct File")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<File<D>, A::Error> {
        let Members { format, version, document, images, profiles } = Members::deserialize(SeqAccessDeserializer::new(seq))?;
        Ok(File { format, version, generator: None, preview: None, profiles, pdf: None, document, images })
    }

    fn visit_map<A: MapAccess<'de>>(mut self, mut map: A) -> Result<File<D>, A::Error> {
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
                "pdfCompatible" if self.pdf.is_some() => {
                    if let Some(state) = self.pdf.as_mut() {
                        state.preference = map.next_value::<Option<bool>>()?;
                    }
                }
                "pdf" if self.pdf.is_some() => {
                    if let Some(state) = self.pdf.as_mut() {
                        state.present = map.next_value::<Option<IgnoredAny>>()?.is_some();
                    }
                }
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
/// `preview` and `pdf`, in the order of the array form. Only [`Contents::read`] makes one.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Contents<D> {
    /// [`EXTENSION`], or [`LEGACY_EXTENSION`] in versions 1 and 2.
    pub format: String,
    /// The file format version.
    pub version: u32,
    /// The document, in the app's own type.
    pub document: D,
    /// The images the document uses, by key.
    pub images: BTreeMap<String, Image>,
    /// ICC files by profile name.
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
        let (file, _) = read(json, false, false)?;
        let File { format, version, document, images, profiles, .. } = file;
        Ok(Self { format, version, document, images, profiles })
    }

    /// Reads the document and its saved PDF preference in one envelope pass, without
    /// keeping or decoding the PDF payload. A non-null `pdf` supplies the fallback when
    /// `pdfCompatible` is absent or null. Repeated members use their last value; arrays
    /// have no PDF preference and return false. A non-boolean preference is an error.
    /// The existing [`Self::read`] continues to ignore that optional preference.
    pub fn read_with_pdf_preference<'de>(json: &'de [u8]) -> Result<(Self, bool), ReadError>
    where
        D: Deserialize<'de>,
    {
        let (file, compatible) = read(json, false, true)?;
        let File { format, version, document, images, profiles, .. } = file;
        Ok((Self { format, version, document, images, profiles }, compatible))
    }

    /// [`File::is_legacy`].
    pub fn is_legacy(&self) -> bool {
        self.format == LEGACY_EXTENSION
    }
}

/// The members of a file besides its document, which is skipped without being decoded. A
/// `preview` or `pdf` of the wrong shape reads as `None`, as in [`File`]; a repeated `preview` or
/// `pdf` is refused.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct Head {
    /// A PNG of the first artboard, when the file has one.
    #[serde(default, deserialize_with = "lenient")]
    pub preview: Option<Image>,
    /// A PDF of every artboard, when the file has one.
    #[serde(default, deserialize_with = "lenient")]
    pub pdf: Option<Image>,
}

/// An entry holding bytes: an image, an ICC profile, the preview or the PDF. Other crates make
/// one with [`Image::inline`], [`Tail::image`] or `Image::default()`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Image {
    /// The media type of the bytes, such as `image/png`.
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

    /// [`Image::len`] with fallible checkpoints while scanning inline data. The checkpoint's
    /// error is returned unchanged.
    pub fn len_with_check<E>(&self, mut check: impl FnMut() -> Result<(), E>) -> Result<u64, E> {
        match self.blob {
            Some([_, len]) => {
                check()?;
                Ok(len)
            }
            None => base64::decoded_len_with_check(&self.data, check),
        }
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

    /// [`Image::bytes_at_most`] with fallible checkpoints during size checks, base64 decoding
    /// and copying. `Ok(None)` has the same meaning as [`Image::bytes_at_most`].
    pub fn bytes_at_most_with_check<E>(&self, tail: &[u8], max: u64, mut check: impl FnMut() -> Result<(), E>) -> Result<Option<Vec<u8>>, E> {
        if self.len_with_check(&mut check)? > max {
            return Ok(None);
        }
        match self.blob {
            Some(at) => {
                let Some(bytes) = entry(tail, at) else { return Ok(None) };
                let mut out = Vec::with_capacity(bytes.len());
                for chunk in bytes.chunks(base64::CHECK_CHUNK) {
                    check()?;
                    out.extend_from_slice(chunk);
                }
                Ok(Some(out))
            }
            None => base64::decode_with_check(&self.data, check),
        }
    }
}

#[cfg(test)]
extern crate self as craft_vectorcraft_format;

#[cfg(test)]
#[path = "../tests/main.rs"]
mod integration;
