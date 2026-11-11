//! The byte container: entries after the JSON, the blob marker, and recognizing a file from its first bytes.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::Arc;

use super::{BLOB_MAGIC, EXTENSION, Error, INLINE_MAX, Image, LEGACY_EXTENSION, TAIL_SINCE, base64};

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
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Part<'a> {
    /// The media type of the bytes, such as `image/png`.
    pub mime: String,
    /// The bytes.
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
