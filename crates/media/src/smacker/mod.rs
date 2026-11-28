//! Clean-room Smacker (`.smk`) demuxer, written from the public community description of the
//! format (the MultimediaWiki "Smacker" page) and checked against real files.
//!
//! A Smacker file is a 104-byte header, a table of frame sizes, a table of frame flags, the
//! Huffman tree data of the video codec, and then the frames. A frame holds an optional palette
//! chunk, one chunk for each audio track that has data in this frame (up to seven tracks), and
//! the video data.
//!
//! [`open`] reads the header and the tables and returns a [`SmackerFile`] with one [`Frame`] per
//! frame. [`SmackerFile::read_frame`] reads a frame's bytes and [`split_frame`] cuts them into
//! [`FrameParts`]. The video and audio decoders are in `craft-video` (`smacker`) and `craft-audio`
//! (`smacker`).
//!
//! Every size and count comes from the file and is treated as hostile: the tables are bounded by
//! the file length, and a frame that runs past the end of the file is dropped
//! ([`SmackerFile::truncated`]).
//!
//! [`ByteTree`] is the byte Huffman tree that the video and the audio decoder share.
//!
//! No dependencies beyond `std`, no `unsafe`, builds for `wasm32-unknown-unknown`.

#![warn(missing_docs)]

#[cfg(test)]
mod tests;
mod tree;

pub use tree::{ByteTree, TreeError};

pub use crate::source::{ByteSource, ReadSeekSource};

use std::io;

/// Bytes of the file header.
pub const HEADER_LEN: usize = 104;
/// Audio tracks a file can hold.
pub const AUDIO_TRACKS: usize = 7;

/// Errors from reading a Smacker file.
#[non_exhaustive]
#[derive(Debug)]
pub enum Error {
    /// Not a Smacker file (no `SMK2` or `SMK4` signature).
    NotSmacker,
    /// A read failed.
    Io(io::Error),
    /// Structurally invalid data.
    Invalid(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotSmacker => f.write_str("not a Smacker file"),
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Invalid(e) => write!(f, "invalid Smacker file: {e}"),
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

/// Whether `head` starts like a Smacker file: `SMK2` or `SMK4`.
pub fn sniff(head: &[u8]) -> bool {
    matches!(head.get(..4), Some(b"SMK2" | b"SMK4"))
}

/// The file signature, which selects the video bitstream version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    /// `SMK2`.
    Smk2,
    /// `SMK4`: full blocks have two more coding modes.
    Smk4,
}

impl Version {
    /// The four signature bytes.
    pub fn signature(self) -> [u8; 4] {
        match self {
            Version::Smk2 => *b"SMK2",
            Version::Smk4 => *b"SMK4",
        }
    }
}

/// How the data of an audio track is coded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioCoding {
    /// Uncompressed PCM: unsigned 8-bit or signed 16-bit little-endian, channels interleaved.
    Pcm,
    /// Smacker's Huffman-coded DPCM.
    Dpcm,
    /// Bink audio, RDFT variant (later Smacker files).
    BinkRdft,
    /// Bink audio, DCT variant (later Smacker files).
    BinkDct,
}

/// One of the seven audio track slots of the header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioTrack {
    /// Samples per second; 0 for an unused slot.
    pub sample_rate: u32,
    /// 16-bit samples (else 8-bit).
    pub sixteen_bit: bool,
    /// Two channels (else one).
    pub stereo: bool,
    /// How the track's chunks are coded.
    pub coding: AudioCoding,
    /// The header's audio size field for this track, in bytes. It does not bound a decoded
    /// chunk. In the real files checked it is the largest decoded size of a chunk in every
    /// `SMK2` track and in most `SMK4` tracks, but in 373 `SMK4` tracks it is the largest stored
    /// size of a chunk, which a DPCM chunk decodes to more than.
    pub size_hint: u32,
    /// The top byte of the header's rate field, as stored.
    pub flags: u8,
}

impl AudioTrack {
    const COMPRESSED: u8 = 0x80;
    const SIXTEEN_BIT: u8 = 0x20;
    const STEREO: u8 = 0x10;
    const BINK_RDFT: u8 = 0x08;
    const BINK_DCT: u8 = 0x04;

    fn parse(rate: u32, size_hint: u32) -> AudioTrack {
        let flags = (rate >> 24) as u8;
        let coding = if flags & Self::BINK_RDFT != 0 {
            AudioCoding::BinkRdft
        } else if flags & Self::BINK_DCT != 0 {
            AudioCoding::BinkDct
        } else if flags & Self::COMPRESSED != 0 {
            AudioCoding::Dpcm
        } else {
            AudioCoding::Pcm
        };
        AudioTrack {
            sample_rate: rate & 0x00FF_FFFF,
            sixteen_bit: flags & Self::SIXTEEN_BIT != 0,
            stereo: flags & Self::STEREO != 0,
            coding,
            size_hint,
            flags,
        }
    }

    /// Whether the slot holds a track (a sample rate other than 0).
    pub fn is_present(&self) -> bool {
        self.sample_rate != 0
    }

    /// 1 or 2.
    pub fn channels(&self) -> u16 {
        if self.stereo { 2 } else { 1 }
    }

    /// 8 or 16.
    pub fn bits(&self) -> u16 {
        if self.sixteen_bit { 16 } else { 8 }
    }

    /// Codec parameters of the track: [`CodecId::SmackerAudio`] for DPCM, [`CodecId::Pcm`] for
    /// uncompressed tracks and [`CodecId::Unknown`] for Bink audio. `pcm` is the layout of the
    /// decoded samples (8-bit is unsigned).
    ///
    /// [`CodecId::SmackerAudio`]: crate::CodecId::SmackerAudio
    /// [`CodecId::Pcm`]: crate::CodecId::Pcm
    /// [`CodecId::Unknown`]: crate::CodecId::Unknown
    pub fn codec_params(&self) -> crate::CodecParams {
        let id = match self.coding {
            AudioCoding::Pcm => crate::CodecId::Pcm,
            AudioCoding::Dpcm => crate::CodecId::SmackerAudio,
            AudioCoding::BinkRdft | AudioCoding::BinkDct => crate::CodecId::Unknown,
        };
        crate::CodecParams::new(id).with_sample_rate(self.sample_rate).with_channels(self.channels()).with_pcm(crate::PcmFormat::int(
            self.bits(),
            self.sixteen_bit,
            false,
        ))
    }
}

/// The 104-byte file header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// `SMK2` or `SMK4`.
    pub version: Version,
    /// Picture width in pixels.
    pub width: u32,
    /// Picture height in pixels (of the coded picture; see [`Header::Y_INTERLACED`] and
    /// [`Header::Y_DOUBLED`]).
    pub height: u32,
    /// Frames, not counting the ring frame.
    pub frames: u32,
    /// The frame period as stored: positive in milliseconds, negative in units of 10 µs, 0 for
    /// the default of 100 ms.
    pub frame_rate: i32,
    /// Header flags ([`Header::RING_FRAME`], [`Header::Y_INTERLACED`], [`Header::Y_DOUBLED`]).
    pub flags: u32,
    /// Bytes of Huffman tree data after the frame tables.
    pub trees_size: u32,
    /// Table sizes of the four video Huffman trees, in the order they are stored: block maps of
    /// mono blocks, colours of mono blocks, full blocks, block types.
    pub tree_sizes: [u32; 4],
    /// The seven audio track slots.
    pub audio: [AudioTrack; AUDIO_TRACKS],
}

impl Header {
    /// Flag: the file ends with one more frame that leads from the last frame back to the first,
    /// for looping ([`SmackerFile::ring_frame`]).
    pub const RING_FRAME: u32 = 1;
    /// Flag: the picture is meant to be shown with its rows interlaced with black rows.
    pub const Y_INTERLACED: u32 = 2;
    /// Flag: the picture is meant to be shown with every row doubled.
    pub const Y_DOUBLED: u32 = 4;

    /// Parses the first [`HEADER_LEN`] bytes of a file.
    pub fn parse(head: &[u8]) -> Result<Header> {
        let version = match head.get(..4) {
            Some(b"SMK2") => Version::Smk2,
            Some(b"SMK4") => Version::Smk4,
            _ => return Err(Error::NotSmacker),
        };
        if head.len() < HEADER_LEN {
            return Err(Error::Invalid("header cut short"));
        }
        let u = |at: usize| le32(head, at);
        let (width, height) = (u(4), u(8));
        if width == 0 || height == 0 {
            return Err(Error::Invalid("zero picture size"));
        }
        let mut audio = [AudioTrack::parse(0, 0); AUDIO_TRACKS];
        for (i, a) in audio.iter_mut().enumerate() {
            *a = AudioTrack::parse(u(72 + 4 * i), u(24 + 4 * i));
        }
        Ok(Header {
            version,
            width,
            height,
            frames: u(12),
            frame_rate: u(16) as i32,
            flags: u(20),
            trees_size: u(52),
            tree_sizes: [u(56), u(60), u(64), u(68)],
            audio,
        })
    }

    /// The frame period as a fraction of a second, `(numerator, denominator)`.
    pub fn frame_period(&self) -> (u32, u32) {
        match self.frame_rate {
            0 => (1, 10),
            ms @ 1.. => (ms.unsigned_abs(), 1000),
            units => (units.unsigned_abs(), 100_000),
        }
    }

    /// Whether the file has a ring frame.
    pub fn has_ring_frame(&self) -> bool {
        self.flags & Self::RING_FRAME != 0
    }
}

/// One frame of the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    /// File offset of the frame's first byte.
    pub offset: u64,
    /// Bytes of the frame.
    pub size: u32,
    /// The key frame flag of the frame size table: the frame does not build on the previous
    /// picture. None of the real `SMK2` files checked sets it, not even on the first frame, so
    /// a clear flag does not say that a frame depends on the one before.
    pub keyframe: bool,
    /// The frame starts with a palette chunk.
    pub has_palette: bool,
    /// Bit `n` is set when the frame holds a chunk of audio track `n`.
    pub audio_mask: u8,
}

/// An opened Smacker file: the header, the frame table and the video Huffman tree data.
#[derive(Clone, Debug)]
pub struct SmackerFile {
    /// The file header.
    pub header: Header,
    /// The frames, in order. Fewer than `header.frames` if the file is cut short.
    pub frames: Vec<Frame>,
    /// The frame after the last one, if the file has one. The header flags it as the frame that
    /// leads back to the start for looping. In 216 of the 243 real files checked, the picture
    /// after it equals the first picture; in the other 27 up to 7% of the pixels differ. It
    /// carries its own chunks, so audio in the first frame is not in it. ffmpeg decodes it as
    /// one more picture at the end.
    pub ring_frame: Option<Frame>,
    /// The Huffman tree data of the video codec (`header.trees_size` bytes).
    pub trees: Vec<u8>,
    /// The file ends before the last frame the header declares.
    pub truncated: bool,
}

/// Opens a Smacker file: reads the header, the frame tables and the Huffman tree data.
pub fn open<S: ByteSource + ?Sized>(src: &S) -> Result<SmackerFile> {
    let len = src.len();
    let mut sig = [0u8; 4];
    if len < 4 {
        return Err(Error::NotSmacker);
    }
    src.read_at(0, &mut sig)?;
    if !sniff(&sig) {
        return Err(Error::NotSmacker);
    }
    if len < HEADER_LEN as u64 {
        return Err(Error::Invalid("header cut short"));
    }
    let mut head = [0u8; HEADER_LEN];
    src.read_at(0, &mut head)?;
    let header = Header::parse(&head)?;

    // five table bytes for each frame, so the file length bounds the count before any allocation
    let total = u64::from(header.frames) + u64::from(header.has_ring_frame());
    let tables = total.checked_mul(5).ok_or(Error::Invalid("frame count"))?;
    let after_tables = (HEADER_LEN as u64).checked_add(tables).ok_or(Error::Invalid("frame count"))?;
    let data_start = after_tables.checked_add(u64::from(header.trees_size)).ok_or(Error::Invalid("tree size"))?;
    if data_start > len {
        return Err(Error::Invalid("frame tables or tree data run past the end of the file"));
    }
    let total = usize::try_from(total).map_err(|_| Error::Invalid("frame count"))?;
    let size_bytes = total.checked_mul(4).ok_or(Error::Invalid("frame count"))?;
    let mut sizes = Vec::new();
    sizes.try_reserve_exact(size_bytes).map_err(|_| Error::Invalid("frame sizes allocation"))?;
    sizes.resize(size_bytes, 0);
    src.read_at(HEADER_LEN as u64, &mut sizes)?;
    let mut types = Vec::new();
    types.try_reserve_exact(total).map_err(|_| Error::Invalid("frame types allocation"))?;
    types.resize(total, 0);
    src.read_at(HEADER_LEN as u64 + sizes.len() as u64, &mut types)?;
    let mut trees = Vec::new();
    // A source length check does not guarantee enough memory for its declared tree data.
    trees.try_reserve_exact(header.trees_size as usize).map_err(|_| Error::Invalid("tree allocation"))?;
    trees.resize(header.trees_size as usize, 0);
    src.read_at(after_tables, &mut trees)?;

    let mut frames = Vec::new();
    // A large, valid source length must not turn an allocation failure into an abort.
    frames.try_reserve_exact(total).map_err(|_| Error::Invalid("frame index allocation"))?;
    let mut offset = data_start;
    let mut truncated = false;
    for (i, &ty) in types.iter().enumerate() {
        let raw = le32(&sizes, 4 * i);
        let size = raw & !3;
        let end = offset.saturating_add(u64::from(size));
        if end > len {
            truncated = true;
            break;
        }
        frames.push(Frame { offset, size, keyframe: raw & 1 != 0, has_palette: ty & 1 != 0, audio_mask: ty >> 1 });
        offset = end;
    }
    let ring_frame = if header.has_ring_frame() && frames.len() == total { frames.pop() } else { None };
    Ok(SmackerFile { header, frames, ring_frame, trees, truncated })
}

impl SmackerFile {
    /// Reads the bytes of `frame` (one of `self.frames`, or the ring frame).
    pub fn read<S: ByteSource + ?Sized>(&self, src: &S, frame: &Frame) -> Result<Vec<u8>> {
        if frame.offset.saturating_add(u64::from(frame.size)) > src.len() {
            return Err(Error::Invalid("frame runs past the end of the file"));
        }
        let mut data = Vec::new();
        // Frame sizes come from the file; report allocation failure to the caller.
        data.try_reserve_exact(frame.size as usize).map_err(|_| Error::Invalid("frame allocation"))?;
        data.resize(frame.size as usize, 0);
        src.read_at(frame.offset, &mut data)?;
        Ok(data)
    }

    /// Reads the bytes of frame `i`.
    pub fn read_frame<S: ByteSource + ?Sized>(&self, src: &S, i: usize) -> Result<Vec<u8>> {
        let frame = self.frames.get(i).ok_or(Error::Invalid("no such frame"))?;
        self.read(src, frame)
    }

    /// Codec parameters of the video: [`CodecId::SmackerVideo`], the picture size, the file
    /// signature as `tag`, and as `extradata` the four tree table sizes (little-endian `u32`s)
    /// followed by the Huffman tree data.
    ///
    /// [`CodecId::SmackerVideo`]: crate::CodecId::SmackerVideo
    pub fn video_params(&self) -> crate::CodecParams {
        let mut extradata = Vec::with_capacity(16 + self.trees.len());
        for s in self.header.tree_sizes {
            extradata.extend_from_slice(&s.to_le_bytes());
        }
        extradata.extend_from_slice(&self.trees);
        crate::CodecParams::new(crate::CodecId::SmackerVideo)
            .with_tag(self.header.version.signature())
            .with_size(self.header.width, self.header.height)
            .with_extradata(extradata)
    }
}

/// The chunks of one frame, borrowed from the frame's bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameParts<'a> {
    /// The palette chunk, without its length byte.
    pub palette: Option<&'a [u8]>,
    /// The chunk of each audio track, without its length field. A compressed chunk starts with
    /// the `u32` size of its decoded samples in bytes.
    pub audio: [Option<&'a [u8]>; AUDIO_TRACKS],
    /// The video data; when empty, the previous palette indices are retained.
    /// A palette chunk still applies and can change the displayed colours.
    pub video: &'a [u8],
}

/// Cuts the bytes of a frame into its palette chunk, audio chunks and video data.
pub fn split_frame<'a>(data: &'a [u8], frame: &Frame) -> Result<FrameParts<'a>> {
    let mut rest = data;
    let mut palette = None;
    if frame.has_palette {
        // the first byte counts 4-byte units, itself included
        let len = usize::from(*rest.first().ok_or(Error::Invalid("palette chunk cut short"))?) * 4;
        if len == 0 || len > rest.len() {
            return Err(Error::Invalid("palette chunk size"));
        }
        let (chunk, tail) = rest.split_at(len);
        palette = chunk.get(1..);
        rest = tail;
    }
    let mut audio = [None; AUDIO_TRACKS];
    for (track, slot) in audio.iter_mut().enumerate() {
        if frame.audio_mask & (1 << track) == 0 {
            continue;
        }
        if rest.len() < 4 {
            return Err(Error::Invalid("audio chunk cut short"));
        }
        // the length counts its own four bytes
        let len = le32(rest, 0) as usize;
        if len < 4 || len > rest.len() {
            return Err(Error::Invalid("audio chunk size"));
        }
        let (chunk, tail) = rest.split_at(len);
        *slot = chunk.get(4..);
        rest = tail;
    }
    Ok(FrameParts { palette, audio, video: rest })
}

/// Little-endian `u32` at `at`; 0 past the end.
fn le32(d: &[u8], at: usize) -> u32 {
    match d.get(at..).and_then(|s| s.first_chunk::<4>()) {
        Some(b) => u32::from_le_bytes(*b),
        None => 0,
    }
}
