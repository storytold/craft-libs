//! Clean-room Smacker video decoder, written from the public community description of the format
//! (the MultimediaWiki "Smacker" page) and checked against real files with ffmpeg as a black-box
//! reference.
//!
//! Smacker video is 8-bit paletted. A picture is coded as 4x4 blocks in raster order, each a
//! change to the previous picture: a mono block (two colours and a 16-bit map), a full block
//! (16 colours), a void block (unchanged) or a solid block (one colour). Block types, colours and
//! maps are symbols of four Huffman trees stored once in the file, and a frame can carry a
//! palette update.
//!
//! ```no_run
//! use craft_media::smacker::{open, split_frame};
//! use craft_video::smacker::Decoder;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let bytes = std::fs::read("intro.smk")?;
//! let file = open(&bytes)?;
//! let mut dec = Decoder::new(&file.video_params())?;
//! for frame in &file.frames {
//!     let data = file.read(&bytes, frame)?;
//!     let parts = split_frame(&data, frame)?;
//!     dec.decode(parts.palette, parts.video)?;
//!     let rgba = dec.to_rgba();
//!     # let _ = rgba;
//! }
//! # Ok(())
//! # }
//! ```
//!
//! The decoder keeps the picture and the palette between frames, so frames are decoded in order
//! from the first one. A file's ring frame (`SmackerFile::ring_frame`) decodes like any other
//! frame, after the last one.

#![warn(missing_docs)]

#[cfg(test)]
mod tests;

use craft_media::bitstream::{BitError, LsbBitReader};
use craft_media::smacker::{ByteTree, TreeError};
use craft_media::{CodecId, CodecParams};

use crate::budget::{DECODE_MEMORY_BUDGET, PictureShape};

/// Decoder errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The data ended inside a tree or a picture.
    #[error("truncated Smacker data")]
    Truncated,
    /// Structurally invalid data.
    #[error("invalid Smacker data: {0}")]
    Invalid(&'static str),
    /// The picture has more than [`MAX_PIXELS`] pixels.
    #[error("Smacker picture of {0}x{1} is over the size limit")]
    TooLarge(u32, u32),
}

impl From<BitError> for Error {
    fn from(_: BitError) -> Self {
        Error::Truncated
    }
}

impl From<TreeError> for Error {
    fn from(e: TreeError) -> Self {
        match e {
            TreeError::Eof => Error::Truncated,
            TreeError::TooLarge => Error::Invalid("byte tree has too many nodes"),
        }
    }
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, Error>;

/// Set on a branch node; the low bits are the index of its right child. The left child is the
/// next node. Leaves hold a 16-bit value.
const BRANCH: u32 = 0x8000_0000;

/// A Huffman tree with 16-bit leaves and a three-entry cache of the values decoded last.
///
/// Three leaf values named in the tree's header are not values but references to that cache: the
/// most recent value, the one before it and the one before that.
#[derive(Clone, Debug)]
struct BigTree {
    nodes: Vec<u32>,
    /// Node indexes of the three cache leaves, most recent value first.
    cache: [usize; 3],
}

impl BigTree {
    /// Reads one of the four trees from the tree data. `size` is the header's table size for
    /// this tree in bytes, which bounds its node count.
    fn read(r: &mut LsbBitReader<'_>, size: u32) -> Result<BigTree> {
        let mut nodes: Vec<u32> = Vec::new();
        let mut cache = [None; 3];
        if r.read_bit()? {
            let low = ByteTree::read_optional(r)?;
            let high = ByteTree::read_optional(r)?;
            let mut escapes = [0u32; 3];
            for e in &mut escapes {
                *e = r.read_bits(16)?;
            }
            // four bytes a node, and never more than a tree over 16-bit values can have: the
            // header's size is hostile, and a node can cost one bit of data but takes far more
            // memory than that
            let limit = (size as usize / 4).saturating_add(4).min(MAX_TREE_NODES);
            // branches whose left subtree is being read
            let mut open: Vec<usize> = Vec::new();
            loop {
                if nodes.len() >= limit {
                    return Err(Error::Invalid("tree has too many nodes"));
                }
                if r.read_bit()? {
                    open.push(nodes.len());
                    nodes.push(BRANCH);
                    continue;
                }
                let mut value = u32::from(low.decode(r)?) | u32::from(high.decode(r)?) << 8;
                // the first escape that names this value makes the leaf a cache leaf; the 0 it
                // then holds is not compared with the escapes after it
                for (slot, &escape) in cache.iter_mut().zip(&escapes) {
                    if value == escape {
                        *slot = Some(nodes.len());
                        value = 0;
                        break;
                    }
                }
                nodes.push(value);
                // a leaf ends the left subtree of the innermost open branch, or the tree
                let Some(branch) = open.pop() else { break };
                let right = u32::try_from(nodes.len()).ok().filter(|&n| n < BRANCH).ok_or(Error::Invalid("tree is too large"))?;
                if let Some(n) = nodes.get_mut(branch) {
                    *n = BRANCH | right;
                }
            }
            r.read_bit()?;
        } else {
            // no tree: every symbol is 0
            nodes.push(0);
        }
        // a cache entry the tree has no leaf for still takes part in the rotation
        let cache = cache.map(|slot| {
            slot.unwrap_or_else(|| {
                nodes.push(0);
                nodes.len() - 1
            })
        });
        Ok(BigTree { nodes, cache })
    }

    /// Clears the cache, as at the start of every frame.
    fn reset(&mut self) {
        for at in self.cache {
            if let Some(n) = self.nodes.get_mut(at) {
                *n = 0;
            }
        }
    }

    fn node(&self, at: usize) -> u32 {
        self.nodes.get(at).copied().unwrap_or(0)
    }

    /// Decodes one 16-bit symbol.
    #[inline]
    fn decode(&mut self, r: &mut LsbBitReader<'_>) -> Result<u16> {
        let mut at = 0usize;
        // every step moves to a later node, so the walk ends
        let value = loop {
            let n = self.node(at);
            if n & BRANCH == 0 {
                break n;
            }
            at = if r.read_bit()? { (n & !BRANCH) as usize } else { at + 1 };
        };
        let [first, second, third] = self.cache;
        if value != self.node(first) {
            let (a, b) = (self.node(first), self.node(second));
            for (at, v) in [(third, b), (second, a), (first, value)] {
                if let Some(n) = self.nodes.get_mut(at) {
                    *n = v;
                }
            }
        }
        Ok(value as u16)
    }
}

/// A tree over 16-bit values has at most 65536 leaves, and so at most 131071 nodes.
const MAX_TREE_NODES: usize = 131_071;

/// The most pixels a picture may have (4096x4096). A frame of a few bytes can fill the whole
/// picture, so the picture size bounds the work a small file can ask for; the memory budget
/// alone would allow gigabytes.
pub const MAX_PIXELS: u64 = 1 << 24;

/// Blocks a block type code covers, by the six-bit run index of the code.
const fn run_length(index: usize) -> usize {
    if index < 59 { index + 1 } else { 128 << (index - 59) }
}

/// An eight-bit level of a six-bit palette component.
const fn expand6(v: u8) -> u8 {
    let v = v & 0x3F;
    (v << 2) | (v >> 4)
}

/// An RGB palette of 256 entries.
pub type Palette = [[u8; 3]; 256];

/// Smacker video decoder: holds the picture and the palette, and applies one frame at a time.
#[derive(Clone, Debug)]
pub struct Decoder {
    width: usize,
    height: usize,
    smk4: bool,
    /// Block maps of mono blocks, colours of mono blocks, full blocks, block types.
    trees: [BigTree; 4],
    palette: Palette,
    /// Palette indexes, `width` a row.
    pixels: Vec<u8>,
}

impl Decoder {
    /// A decoder for the video described by `params`
    /// (`craft_media::smacker::SmackerFile::video_params`): the picture size, the file signature
    /// as `tag` (`SMK2` or `SMK4`), and as `extradata` the four tree table sizes followed by the
    /// Huffman tree data.
    pub fn new(params: &CodecParams) -> Result<Decoder> {
        if params.codec != CodecId::SmackerVideo {
            return Err(Error::Invalid("not Smacker video"));
        }
        let (Some(width), Some(height)) = (params.width, params.height) else { return Err(Error::Invalid("no picture size")) };
        let smk4 = match params.tag {
            Some(tag) if &tag == b"SMK2" => false,
            Some(tag) if &tag == b"SMK4" => true,
            _ => return Err(Error::Invalid("unknown version")),
        };
        let shape =
            PictureShape { width: u64::from(width), height: u64::from(height), shift_x: 0, shift_y: 0, chroma_planes: 0, bytes_per_sample: 1 };
        if !shape.fits(1, DECODE_MEMORY_BUDGET.min(MAX_PIXELS)) {
            return Err(Error::TooLarge(width, height));
        }
        let (Ok(w), Ok(h)) = (usize::try_from(width), usize::try_from(height)) else { return Err(Error::TooLarge(width, height)) };
        let len = w.checked_mul(h).ok_or(Error::TooLarge(width, height))?;

        let (sizes, data) = params.extradata.split_at_checked(16).ok_or(Error::Truncated)?;
        let size = |i: usize| sizes.get(4 * i..).and_then(|s| s.first_chunk::<4>()).map_or(0, |b| u32::from_le_bytes(*b));
        let mut r = LsbBitReader::new(data);
        let trees =
            [BigTree::read(&mut r, size(0))?, BigTree::read(&mut r, size(1))?, BigTree::read(&mut r, size(2))?, BigTree::read(&mut r, size(3))?];
        Ok(Decoder { width: w, height: h, smk4, trees, palette: [[0; 3]; 256], pixels: vec![0; len] })
    }

    /// Picture width in pixels.
    pub fn width(&self) -> u32 {
        self.width as u32
    }

    /// Picture height in pixels.
    pub fn height(&self) -> u32 {
        self.height as u32
    }

    /// The current picture as palette indexes, `width` a row, top row first.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// The current palette.
    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    /// The current picture as RGBA bytes (alpha 255), top row first.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len().saturating_mul(4));
        for &p in &self.pixels {
            let [r, g, b] = self.palette[usize::from(p)];
            out.extend_from_slice(&[r, g, b, 255]);
        }
        out
    }

    /// Applies one frame: its palette chunk, if it has one, and its video data. Empty video data
    /// leaves the picture as it is.
    ///
    /// A palette chunk with a copy record that runs past entry 255 is an error, and the frame's
    /// video data is then not applied. After an error in the video data the picture holds the
    /// blocks decoded before it.
    pub fn decode(&mut self, palette: Option<&[u8]>, video: &[u8]) -> Result<()> {
        if let Some(chunk) = palette {
            self.palette = apply_palette(&self.palette, chunk)?;
        }
        if video.is_empty() {
            return Ok(());
        }
        self.decode_picture(video)
    }

    fn decode_picture(&mut self, video: &[u8]) -> Result<()> {
        for t in &mut self.trees {
            t.reset();
        }
        let [map_tree, colour_tree, full_tree, type_tree] = &mut self.trees;
        let (bw, bh) = (self.width / 4, self.height / 4);
        let blocks = bw.saturating_mul(bh);
        let mut out = Blocks { pixels: &mut self.pixels, stride: self.width, bw };
        let mut r = LsbBitReader::new(video);
        let mut blk = 0usize;
        while blk < blocks {
            let code = type_tree.decode(&mut r)?;
            let run = run_length(usize::from(code >> 2) & 0x3F).min(blocks - blk);
            match code & 3 {
                // mono: two colours, and a map with a bit for each pixel
                0 => {
                    for _ in 0..run {
                        let colours = colour_tree.decode(&mut r)?.to_le_bytes();
                        let mut map = map_tree.decode(&mut r)?;
                        let mut px = [0u8; 16];
                        for p in &mut px {
                            *p = colours[usize::from(map & 1)];
                            map >>= 1;
                        }
                        out.put(blk, &px);
                        blk += 1;
                    }
                }
                // full: sixteen colours
                1 => {
                    let mode = if !self.smk4 {
                        0
                    } else if r.read_bit()? {
                        1
                    } else if r.read_bit()? {
                        2
                    } else {
                        0
                    };
                    for _ in 0..run {
                        let mut px = [0u8; 16];
                        match mode {
                            // a pair of colours a code: the right pair of a row before the left one
                            0 => {
                                for row in px.as_chunks_mut::<4>().0 {
                                    let right = full_tree.decode(&mut r)?.to_le_bytes();
                                    let left = full_tree.decode(&mut r)?.to_le_bytes();
                                    *row = [left[0], left[1], right[0], right[1]];
                                }
                            }
                            // double: 2x2 pixels a colour, two colours a code
                            1 => {
                                for rows in px.as_chunks_mut::<8>().0 {
                                    let [a, b] = full_tree.decode(&mut r)?.to_le_bytes();
                                    *rows = [a, a, b, b, a, a, b, b];
                                }
                            }
                            // each row doubled down
                            _ => {
                                for rows in px.as_chunks_mut::<8>().0 {
                                    let right = full_tree.decode(&mut r)?.to_le_bytes();
                                    let left = full_tree.decode(&mut r)?.to_le_bytes();
                                    *rows = [left[0], left[1], right[0], right[1], left[0], left[1], right[0], right[1]];
                                }
                            }
                        }
                        out.put(blk, &px);
                        blk += 1;
                    }
                }
                // void: unchanged
                2 => blk += run,
                // solid: one colour, in the high byte of the code
                _ => {
                    let px = [(code >> 8) as u8; 16];
                    for _ in 0..run {
                        out.put(blk, &px);
                        blk += 1;
                    }
                }
            }
        }
        Ok(())
    }
}

/// The picture as a grid of 4x4 blocks.
struct Blocks<'a> {
    pixels: &'a mut [u8],
    stride: usize,
    /// Blocks a row.
    bw: usize,
}

impl Blocks<'_> {
    /// Writes block `blk` (in raster order) from 16 pixels in raster order.
    #[inline]
    fn put(&mut self, blk: usize, px: &[u8; 16]) {
        if self.bw == 0 {
            return;
        }
        let (bx, by) = (blk % self.bw, blk / self.bw);
        for (row, src) in px.as_chunks::<4>().0.iter().enumerate() {
            let at = (by * 4 + row) * self.stride + bx * 4;
            if let Some(dst) = self.pixels.get_mut(at..at + 4) {
                dst.copy_from_slice(src);
            }
        }
    }
}

/// The palette that the palette chunk `chunk` makes of `previous`.
///
/// The chunk is a list of records that fill the 256 entries in order: a run of entries kept
/// from the previous palette, a run copied from another position of the previous palette, or
/// one entry of three six-bit components. A chunk that ends before entry 255 leaves the entries
/// after it as they were; a record cut short by the end of the chunk is dropped. This follows
/// the format description. ffmpeg differs here: it reads on past the chunk into the frame's
/// other data until it has 256 entries. None of the real files checked has such a chunk.
fn apply_palette(previous: &Palette, chunk: &[u8]) -> Result<Palette> {
    let mut palette = *previous;
    let mut bytes = chunk.iter().copied();
    let mut at = 0usize;
    while at < 256 {
        let Some(b) = bytes.next() else { break };
        if b & 0x80 != 0 {
            at += usize::from(b & 0x7F) + 1;
        } else if b & 0x40 != 0 {
            let count = usize::from(b & 0x3F) + 1;
            let Some(from) = bytes.next().map(usize::from) else { break };
            let src = previous.get(from..from + count).ok_or(Error::Invalid("palette copy runs past entry 255"))?;
            let dst = palette.get_mut(at..at + count).ok_or(Error::Invalid("palette copy runs past entry 255"))?;
            dst.copy_from_slice(src);
            at += count;
        } else {
            let (Some(g), Some(bl)) = (bytes.next(), bytes.next()) else { break };
            if let Some(entry) = palette.get_mut(at) {
                *entry = [expand6(b), expand6(g), expand6(bl)];
            }
            at += 1;
        }
    }
    Ok(palette)
}
