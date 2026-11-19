//! Clean-room Sierra VMD (Video and Music Data) demuxer, written from the public format
//! description at <https://wiki.multimedia.cx/index.php/VMD> ("Description of the Sierra Video
//! and Music Data (VMD) Format", Mike Melanson and Vladimir Gneushev) and checked against
//! ffmpeg's reading of files made by this crate's tests.
//!
//! A VMD file is an 816-byte header (picture size, a 256-colour palette, the sound format), the
//! frame data, and a table of contents at the end: one 6-byte offset record a block, then one
//! 16-byte record a frame. A block is one picture period and holds the same number of frames in
//! every block: usually a sound frame and a video frame.
//!
//! Pictures and sound are timed differently. The video frame of block `n` is shown at picture
//! period `n` ([`Frame::block`], [`VmdFile::frame_rate`]). Sound is one continuous stream that
//! starts with the first picture: decode the sound frames in file order and play the samples end
//! to end. The first sound frame holds many blocks of sound, which the player loads ahead, so
//! each later sound frame is heard that much later than the block it is stored in, and the sound
//! runs on after the last picture.
//!
//! [`open`] reads the header and the table of contents; [`VmdFile::read_frame`] reads one frame's
//! data on demand. The video decoder is `craft_video::vmd` and the sound decoder
//! `craft_audio::vmd`; both take the header ([`VmdFile::header_bytes`]) as their configuration.
//!
//! Only the 816-byte header is read: the later Coktel Vision revisions with a longer header or
//! one without a palette are refused as [`Error::NotVmd`]. Files whose pictures are 16-bit,
//! 24-bit or Indeo 3 open, and say so in [`Header::video_format`].
//!
//! Every size and count comes from the file and is treated as hostile: the table of contents
//! must lie inside the file, its length is capped, and a frame is read only if all of it is
//! there.
//!
//! No dependencies beyond `std`, no `unsafe`, builds for `wasm32-unknown-unknown`.

#![warn(missing_docs)]

#[cfg(test)]
mod tests;

pub use crate::source::{ByteSource, ReadSeekSource};

use std::io;

/// Errors from reading a VMD file.
#[non_exhaustive]
#[derive(Debug)]
pub enum Error {
    /// Not a VMD file with the 816-byte header.
    NotVmd,
    /// A read failed.
    Io(io::Error),
    /// Structurally invalid data.
    Invalid(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotVmd => f.write_str("not a VMD file"),
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Invalid(e) => write!(f, "invalid VMD: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, Error>;

/// Bytes of the file header, its two-byte length field included.
pub const HEADER_LEN: usize = 816;

/// Frames a table of contents may list. A file has at most 65 535 blocks of a few frames each;
/// this bounds the table a hostile header can ask for (16 bytes a frame).
const MAX_FRAMES: u64 = 1 << 22;

/// Whether `head` starts like a VMD file: the header length field, 0x32E. Two bytes are a weak
/// signature, so check the file extension as well, and expect [`open`] to refuse a false match.
pub fn sniff(head: &[u8]) -> bool {
    head.get(..2) == Some(&[0x2E, 0x03])
}

/// How the pictures are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoFormat {
    /// 8-bit palette indices: what `craft_video::vmd` decodes.
    Paletted,
    /// 16-bit RGB (codec version 13).
    Rgb16,
    /// 24-bit RGB (codec version 5).
    Rgb24,
    /// Intel Indeo 3 (codec version 7, or the `iv32` mark).
    Indeo3,
}

/// The sound format, from the header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioHeader {
    /// Samples per second.
    pub sample_rate: u16,
    /// The frame length field, sign removed: samples a block and channel, except in old-style
    /// stereo, where it counts both channels.
    pub frame_length: u16,
    /// 8 (unsigned PCM) or 16 (DPCM).
    pub bits: u8,
    /// 1 or 2.
    pub channels: u8,
    /// Number of sound buffers. In the game files seen it is one more than the blocks of sound
    /// (silent ones included) in the file's first sound frame.
    pub buffers: u16,
    /// The audio flags as stored (0x8000 old-style stereo, 0x200 new-style stereo, 0x10 IMA
    /// ADPCM).
    pub flags: u16,
}

impl AudioHeader {
    /// Samples of one block, for one channel.
    pub fn block_samples(&self) -> u32 {
        let n = u32::from(self.frame_length);
        if self.flags & 0x8000 != 0 { n / 2 } else { n }
    }

    /// Whether the sound is IMA ADPCM, which `craft_audio::vmd` does not decode.
    pub fn is_ima_adpcm(&self) -> bool {
        self.flags & 0x10 != 0
    }
}

/// The file header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// Bytes 2-3: the VMD handle and version.
    pub version: u16,
    /// Bytes 4-5: the video codec version.
    pub codec_version: u16,
    /// Number of blocks.
    pub blocks: u16,
    /// Top coordinate of the picture on the screen.
    pub top: u16,
    /// Left coordinate of the picture on the screen.
    pub left: u16,
    /// Picture width in pixels.
    pub width: u16,
    /// Picture height in pixels.
    pub height: u16,
    /// Flags (0x1000: the file has sound, if its sample rate is not 0).
    pub flags: u16,
    /// Frames in every block.
    pub frames_per_block: u16,
    /// Offset of the frame data.
    pub data_offset: u32,
    /// Bytes 24-27: `iv32` (in either case) when the video is Indeo 3.
    pub indeo: [u8; 4],
    /// The first palette: 256 R-G-B triplets of 6-bit components (0 to 63).
    pub palette: [u8; 768],
    /// Recommended size of the buffer a frame is loaded into.
    pub load_buffer_size: u32,
    /// Size of the buffer LZ-packed video unpacks into.
    pub unpack_buffer_size: u32,
    /// The sound format, if the file has sound.
    pub audio: Option<AudioHeader>,
    /// Offset of the table of contents.
    pub toc_offset: u32,
}

impl Header {
    /// How the pictures are stored.
    pub fn video_format(&self) -> VideoFormat {
        if self.indeo.eq_ignore_ascii_case(b"iv32") {
            return VideoFormat::Indeo3;
        }
        match self.codec_version {
            5 => VideoFormat::Rgb24,
            7 => VideoFormat::Indeo3,
            13 => VideoFormat::Rgb16,
            _ => VideoFormat::Paletted,
        }
    }

    fn parse(h: &[u8; HEADER_LEN]) -> Result<Header> {
        let u16_at = |at: usize| u16::from_le_bytes([h[at], h[at + 1]]);
        let u32_at = |at: usize| u32::from_le_bytes([h[at], h[at + 1], h[at + 2], h[at + 3]]);
        if usize::from(u16_at(0)) != HEADER_LEN - 2 {
            return Err(Error::NotVmd);
        }
        let flags = u16_at(16);
        // the flag is the description's rule. A sample rate of 0 is no sound either: ffmpeg goes
        // by the rate alone, and also plays a file that has a rate without the flag
        let audio = (flags & 0x1000 != 0 && u16_at(804) != 0).then(|| {
            let length = u16_at(806) as i16;
            let audio_flags = u16_at(810);
            AudioHeader {
                sample_rate: u16_at(804),
                frame_length: length.unsigned_abs(),
                bits: if length < 0 { 16 } else { 8 },
                channels: if audio_flags & 0x8200 != 0 { 2 } else { 1 },
                buffers: u16_at(808),
                flags: audio_flags,
            }
        });
        let mut palette = [0; 768];
        palette.copy_from_slice(&h[28..796]);
        Ok(Header {
            version: u16_at(2),
            codec_version: u16_at(4),
            blocks: u16_at(6),
            top: u16_at(8),
            left: u16_at(10),
            width: u16_at(12),
            height: u16_at(14),
            flags,
            frames_per_block: u16_at(18),
            data_offset: u32_at(20),
            indeo: [h[24], h[25], h[26], h[27]],
            palette,
            load_buffer_size: u32_at(796),
            unpack_buffer_size: u32_at(800),
            audio,
            toc_offset: u32_at(812),
        })
    }
}

/// What a frame holds, from its record in the table of contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameKind {
    /// Sound.
    Audio {
        /// 1: one block of sound; 2: a silence mask, then blocks of sound; 3: one block of
        /// silence.
        flags: u8,
    },
    /// A picture update. The coordinates are as stored, and include the last column and row;
    /// [`VmdFile::picture_rect`] places them in the picture.
    Video {
        /// Left column.
        left: u16,
        /// Top row.
        top: u16,
        /// Right column.
        right: u16,
        /// Bottom row.
        bottom: u16,
        /// The data begins with a new palette.
        new_palette: bool,
    },
    /// A frame type this module does not know.
    Other(u8),
}

/// One frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    /// What the frame holds.
    pub kind: FrameKind,
    /// The block the frame is stored in. A block is one picture period, so this is when a video
    /// frame is shown. It is not when a sound frame is heard: see the module documentation.
    pub block: u32,
    /// Offset of the frame data.
    pub offset: u64,
    /// Bytes of frame data; 0 for a frame that changes nothing.
    pub size: u32,
}

/// An opened VMD file: the header and the frames of every block, in file order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VmdFile {
    /// The header.
    pub header: Header,
    /// The frames, block after block.
    pub frames: Vec<Frame>,
    raw_header: Vec<u8>,
    /// For each frame, the picture's screen position when the frame is decoded.
    origins: Vec<(u16, u16)>,
}

/// Open a VMD file: read the header and the table of contents.
pub fn open<S: ByteSource + ?Sized>(src: &S) -> Result<VmdFile> {
    let mut raw = [0u8; HEADER_LEN];
    if src.len() < HEADER_LEN as u64 {
        return Err(Error::NotVmd);
    }
    src.read_at(0, &mut raw)?;
    let header = Header::parse(&raw)?;

    let blocks = u64::from(header.blocks);
    let per_block = u64::from(header.frames_per_block);
    let count = blocks * per_block;
    if count > MAX_FRAMES {
        return Err(Error::Invalid(format!("{count} frames in the table of contents")));
    }
    let toc_len = blocks * 6 + count * 16;
    let toc_offset = u64::from(header.toc_offset);
    if toc_offset < HEADER_LEN as u64 || toc_offset.checked_add(toc_len).is_none_or(|end| end > src.len()) {
        return Err(Error::Invalid("the table of contents is outside the file".into()));
    }
    // bounded by MAX_FRAMES and by the file length
    let mut toc = vec![0u8; toc_len as usize];
    src.read_at(toc_offset, &mut toc)?;
    let (offsets, records) = toc.split_at(blocks as usize * 6);

    let mut frames = Vec::with_capacity(count as usize);
    for (block, (entry, records)) in offsets.as_chunks::<6>().0.iter().zip(records.chunks_exact((per_block as usize * 16).max(1))).enumerate() {
        let mut offset = u64::from(u32::from_le_bytes([entry[2], entry[3], entry[4], entry[5]]));
        for r in records.as_chunks::<16>().0 {
            let size = u32::from_le_bytes([r[2], r[3], r[4], r[5]]);
            let u16_at = |at: usize| u16::from_le_bytes([r[at], r[at + 1]]);
            let kind = match r[0] {
                1 => FrameKind::Audio { flags: r[6] },
                2 => FrameKind::Video { left: u16_at(6), top: u16_at(8), right: u16_at(10), bottom: u16_at(12), new_palette: r[15] & 2 != 0 },
                other => FrameKind::Other(other),
            };
            frames.push(Frame { kind, block: block as u32, offset, size });
            // at most 65 535 frames of under 4 GiB each: no overflow
            offset += u64::from(size);
        }
    }
    // the picture's place on the screen as each frame finds it: see `picture_rect`
    let mut origin = (0, 0);
    let origins = frames
        .iter()
        .map(|f| {
            if let FrameKind::Video { left, top, right, bottom, .. } = f.kind {
                let full = right.checked_sub(left).and_then(|w| w.checked_add(1)) == Some(header.width)
                    && bottom.checked_sub(top).and_then(|h| h.checked_add(1)) == Some(header.height);
                if full && f.size > 0 && (left, top) != (0, 0) {
                    origin = (left, top);
                }
            }
            origin
        })
        .collect();
    Ok(VmdFile { header, frames, raw_header: raw.to_vec(), origins })
}

/// `size` zero bytes, or `None` if they cannot be had. `vec![0; size]` panics with a capacity
/// overflow for a size the address space cannot hold, which a frame of over 2 GiB is on a 32-bit
/// target.
fn zeroed(size: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(size).ok()?;
    bytes.resize(size, 0);
    Some(bytes)
}

impl VmdFile {
    /// The 816 header bytes as stored: the configuration (`CodecParams::extradata`) of the VMD
    /// video and sound decoders.
    pub fn header_bytes(&self) -> &[u8] {
        &self.raw_header
    }

    /// Read the data of frame `index`. A frame that reaches past the end of the file is an
    /// error.
    pub fn read_frame<S: ByteSource + ?Sized>(&self, src: &S, index: usize) -> Result<Vec<u8>> {
        let f = self.frames.get(index).ok_or_else(|| Error::Invalid(format!("no frame {index}")))?;
        // checked before allocating: the size is the file's word
        if f.offset.checked_add(u64::from(f.size)).is_none_or(|end| end > src.len()) {
            return Err(Error::Invalid(format!("frame {index} reaches past the end of the file")));
        }
        let mut data = zeroed(f.size as usize).ok_or_else(|| Error::Invalid(format!("frame {index} is too large to load")))?;
        src.read_at(f.offset, &mut data)?;
        Ok(data)
    }

    /// Pictures a second, as a fraction. A file with sound plays one block of sound a picture;
    /// one without plays 10 pictures a second.
    pub fn frame_rate(&self) -> (u32, u32) {
        match &self.header.audio {
            Some(a) if a.block_samples() > 0 => (u32::from(a.sample_rate), a.block_samples()),
            _ => (10, 1),
        }
    }

    /// Where the video frame `index` updates the picture: `(left, top, right, bottom)` with the
    /// last column and row included, or `None` for a frame that is not video or lies outside the
    /// picture.
    ///
    /// Frame coordinates are screen coordinates, and a picture need not sit at the screen's
    /// corner. The description does not say where the picture's position comes from. This does
    /// what ffmpeg was seen to do: the picture starts at the corner, and a video frame that has
    /// data, is exactly as large as the picture and is not at the corner moves it there for that
    /// frame and those after it. The position is taken off the frame's coordinates. The header's
    /// own position fields are not used.
    pub fn picture_rect(&self, index: usize) -> Option<(u16, u16, u16, u16)> {
        let FrameKind::Video { left, top, right, bottom, .. } = self.frames.get(index)?.kind else { return None };
        let h = &self.header;
        let (x, y) = *self.origins.get(index)?;
        let (left, right) = (left.checked_sub(x)?, right.checked_sub(x)?);
        let (top, bottom) = (top.checked_sub(y)?, bottom.checked_sub(y)?);
        (left <= right && top <= bottom && right < h.width && bottom < h.height).then_some((left, top, right, bottom))
    }

    /// What a video decoder needs, in the container-neutral form the codec crates take:
    /// [`CodecId::VmdVideo`] for paletted pictures, with the header as `extradata`. 16-bit,
    /// 24-bit and Indeo 3 pictures are [`CodecId::Unknown`] (Indeo 3 with the tag `IV32`).
    ///
    /// [`CodecId::VmdVideo`]: crate::CodecId::VmdVideo
    /// [`CodecId::Unknown`]: crate::CodecId::Unknown
    pub fn video_params(&self) -> crate::CodecParams {
        use crate::{CodecId, CodecParams};
        let h = &self.header;
        let params = match h.video_format() {
            VideoFormat::Paletted => CodecParams::new(CodecId::VmdVideo).with_extradata(self.raw_header.clone()),
            VideoFormat::Indeo3 => CodecParams::new(CodecId::Unknown).with_tag(*b"IV32"),
            VideoFormat::Rgb16 | VideoFormat::Rgb24 => CodecParams::new(CodecId::Unknown),
        };
        params.with_size(u32::from(h.width), u32::from(h.height))
    }

    /// What a sound decoder needs, or `None` for a file without sound: [`CodecId::VmdAudio`]
    /// with the header as `extradata`, or [`CodecId::Unknown`] for IMA ADPCM sound.
    ///
    /// [`CodecId::VmdAudio`]: crate::CodecId::VmdAudio
    /// [`CodecId::Unknown`]: crate::CodecId::Unknown
    pub fn audio_params(&self) -> Option<crate::CodecParams> {
        use crate::{CodecId, CodecParams};
        let a = self.header.audio.as_ref()?;
        let params = if a.is_ima_adpcm() {
            CodecParams::new(CodecId::Unknown)
        } else {
            CodecParams::new(CodecId::VmdAudio).with_extradata(self.raw_header.clone())
        };
        Some(params.with_sample_rate(u32::from(a.sample_rate)).with_channels(u16::from(a.channels)))
    }
}
