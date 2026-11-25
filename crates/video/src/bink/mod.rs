//! Clean-room pure-Rust Bink video decoder (Bink 1, revisions `f` and `i`).
//!
//! Bink has no published specification. This decoder was written from the public description of
//! the format's outline on the MultimediaWiki and from black-box observation of how an existing
//! decoder (ffmpeg, as a program) responds to real and hand-built streams; no implementation
//! was read. `README.md` in this directory states each rule and how it was established.
//!
//! [`Decoder`] takes the video packet of each frame, as `craft_media::bink` splits it out of a
//! `.bik` file, and returns the picture as 8-bit 4:2:0 planes.
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut dec = craft_video::bink::Decoder::new(640, 480, *b"BIKi", 0)?;
//! let packet: Vec<u8> = Vec::new(); // the video packet of one frame
//! let picture = dec.decode(&packet)?;
//! println!("{}x{}", picture.width, picture.height);
//! # Ok(())
//! # }
//! ```
//!
//! Not supported, and refused when the decoder is made: revisions other than `f` and `i`, alpha
//! planes and grayscale video. None could be checked against a sample.
//!
//! Every count and size in a packet is treated as hostile: reads are bounded by the packet,
//! tables by the widths of their length fields, and pictures by the decoder memory budget.

#![warn(missing_docs)]

mod bits;
mod decode;
#[rustfmt::skip]
mod tables;
#[cfg(test)]
mod tests;

use bits::Bits;

/// Errors from decoding Bink video.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The packet is not valid Bink video, or ends early.
    #[error("invalid Bink video: {0}")]
    Invalid(String),
    /// A part of the format this decoder does not handle.
    #[error("unsupported Bink video: {0}")]
    Unsupported(String),
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, Error>;

/// The largest width or height a Bink file can declare.
const MAX_DIMENSION: u32 = 32767;

/// The shortest packet that can hold one plane's 23 tree headers.
const MIN_PACKET_LEN: usize = 12;

/// Video flag: frames carry an alpha plane.
const FLAG_ALPHA: u32 = 1 << 20;
/// Video flag: the video is grayscale.
const FLAG_GRAYSCALE: u32 = 1 << 17;

/// A decoded picture: 8-bit 4:2:0, planes tight (stride = width).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    /// Luma width in samples.
    pub width: u32,
    /// Luma height in samples.
    pub height: u32,
    /// Chroma width in samples: half the luma width, rounded up.
    pub chroma_width: u32,
    /// Chroma height in samples: half the luma height, rounded up.
    pub chroma_height: u32,
    /// Luma samples.
    pub y: Vec<u8>,
    /// Cb samples.
    pub u: Vec<u8>,
    /// Cr samples.
    pub v: Vec<u8>,
}

/// One plane's sample store: the picture being decoded and the previous one, both padded to
/// whole 8x8 blocks.
struct PlaneStore {
    cur: Vec<u8>,
    prev: Vec<u8>,
    width: usize,
    height: usize,
    blocks_wide: usize,
    blocks_high: usize,
    /// The most 16x16 blocks a row can hold.
    sub_types: usize,
}

impl PlaneStore {
    /// A plane of `width` by `height` samples; `sub_types` is the count that sizes the sub-type
    /// length field.
    fn new(width: usize, height: usize, sub_types: usize) -> PlaneStore {
        let (blocks_wide, blocks_high) = (width.div_ceil(8), height.div_ceil(8));
        let samples = blocks_wide * 8 * blocks_high * 8;
        PlaneStore { cur: vec![0; samples], prev: vec![0; samples], width, height, blocks_wide, blocks_high, sub_types }
    }

    /// The visible samples of the current picture, rows of `width`.
    fn visible(&self) -> Vec<u8> {
        let stride = self.blocks_wide * 8;
        let mut out = Vec::with_capacity(self.width * self.height);
        for row in self.cur.chunks_exact(stride).take(self.height) {
            out.extend_from_slice(&row[..self.width]);
        }
        out
    }
}

/// A Bink video decoder for one stream. Frames are decoded in order: every frame but a key
/// frame builds on the picture before it.
pub struct Decoder {
    width: u32,
    height: u32,
    /// Whether the packet starts with a 32-bit size word before the luma plane (revision `i`).
    size_word: bool,
    /// Whether colours are a sign bit and a 7-bit magnitude around 128 (revision `f`).
    signed_colours: bool,
    /// Planes in the order luma, Cb, Cr.
    planes: [PlaneStore; 3],
    /// The index into `planes` of each coded plane, in stream order.
    coded_order: [usize; 3],
    /// Whether a picture has been decoded since the decoder was made or reset.
    has_previous: bool,
}

impl Decoder {
    /// A decoder for video of `width` by `height` samples. `signature` is the first four bytes
    /// of the file (`BIK` and the revision letter) and `video_flags` the header's video flags.
    pub fn new(width: u32, height: u32, signature: [u8; 4], video_flags: u32) -> Result<Decoder> {
        if &signature[..3] != b"BIK" {
            return Err(Error::Invalid("not a Bink 1 signature".into()));
        }
        // the coded order of the chroma planes differs between the two revisions
        let (size_word, coded_order) = match signature[3] {
            b'i' => (true, [0, 2, 1]),
            b'f' => (false, [0, 1, 2]),
            r => return Err(Error::Unsupported(format!("revision '{}'", char::from(r).escape_default()))),
        };
        if video_flags & FLAG_ALPHA != 0 {
            return Err(Error::Unsupported("alpha plane".into()));
        }
        if video_flags & FLAG_GRAYSCALE != 0 {
            return Err(Error::Unsupported("grayscale video".into()));
        }
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(Error::Invalid(format!("picture size {width}x{height}")));
        }
        // two padded pictures are held, and a third is made for each decoded frame
        let shape = crate::budget::PictureShape {
            width: u64::from(width).div_ceil(8) * 8 + 8,
            height: u64::from(height).div_ceil(8) * 8 + 8,
            shift_x: 1,
            shift_y: 1,
            chroma_planes: 2,
            bytes_per_sample: 1,
        };
        if !shape.fits(3, crate::budget::DECODE_MEMORY_BUDGET) {
            return Err(Error::Unsupported(format!("picture size {width}x{height} exceeds the decoder memory budget")));
        }
        let (w, h) = (width as usize, height as usize);
        // the sub-type field of a chroma plane is sized from half the luma width rounded down,
        // although the plane itself is half the luma width rounded up
        let chroma = || PlaneStore::new(w.div_ceil(2), h.div_ceil(2), (w / 2 + 7) / 16);
        Ok(Decoder {
            width,
            height,
            size_word,
            signed_colours: !size_word,
            planes: [PlaneStore::new(w, h, (w + 7) / 16), chroma(), chroma()],
            coded_order,
            has_previous: false,
        })
    }

    /// A decoder for the video of a Bink file, from `craft_media::bink::BinkFile::video_params`:
    /// the picture size, the signature as `tag` and the video flags as `extradata`.
    pub fn from_params(params: &craft_media::CodecParams) -> Result<Decoder> {
        let (Some(width), Some(height)) = (params.width, params.height) else {
            return Err(Error::Invalid("no picture size".into()));
        };
        let signature = params.tag.ok_or_else(|| Error::Invalid("no signature".into()))?;
        let flags = match params.extradata.get(..4) {
            Some(&[a, b, c, d]) => u32::from_le_bytes([a, b, c, d]),
            _ => 0,
        };
        Decoder::new(width, height, signature, flags)
    }

    /// Forgets the previous picture, as before the first frame. Use it before decoding from a
    /// key frame after a seek.
    pub fn reset(&mut self) {
        for p in &mut self.planes {
            p.cur.fill(0);
            p.prev.fill(0);
        }
        self.has_previous = false;
    }

    /// Decodes the video packet of the next frame. After an error the decoder still holds the
    /// last picture that decoded, so later frames build on that one.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Picture> {
        // the tree headers of one plane alone take 92 bits; refuse less before touching a picture
        if packet.len() < MIN_PACKET_LEN {
            return Err(Error::Invalid("the packet is too short to hold a plane".into()));
        }
        // A new picture starts as zeros, so a block no entry draws is 0. ffmpeg leaves such a
        // block as its recycled buffer held it (zeros, or some earlier picture), which is not a
        // rule of the format; a real stream draws every block.
        for p in &mut self.planes {
            std::mem::swap(&mut p.cur, &mut p.prev);
            p.cur.fill(0);
        }
        if let Err(e) = self.decode_planes(packet) {
            // the last picture is current again; the failed one is cleared when the next frame starts
            for p in &mut self.planes {
                std::mem::swap(&mut p.cur, &mut p.prev);
            }
            return Err(e);
        }
        self.has_previous = true;
        Ok(Picture {
            width: self.width,
            height: self.height,
            chroma_width: self.width.div_ceil(2),
            chroma_height: self.height.div_ceil(2),
            y: self.planes[0].visible(),
            u: self.planes[1].visible(),
            v: self.planes[2].visible(),
        })
    }

    fn decode_planes(&mut self, packet: &[u8]) -> Result<()> {
        let mut bits = Bits::new(packet);
        if self.size_word {
            // the size of the luma plane's data; planes are found by alignment instead
            bits.skip(32)?;
        }
        for index in self.coded_order {
            let p = &mut self.planes[index];
            let mut plane = decode::Plane {
                cur: &mut p.cur,
                prev: self.has_previous.then_some(&p.prev[..]),
                blocks_wide: p.blocks_wide,
                blocks_high: p.blocks_high,
                sub_types: p.sub_types,
            };
            decode::decode_plane(&mut bits, &mut plane, self.signed_colours)?;
            bits.align32();
        }
        Ok(())
    }
}
