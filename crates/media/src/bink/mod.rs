//! Clean-room Bink (`.bik`) demuxer for Bink 1 files.
//!
//! Bink has no published specification. The layout here follows the public description of the
//! container on the MultimediaWiki (`wiki.multimedia.cx`, "Bink Container") and was checked
//! against what ffprobe reports for real files: frame count, key frames and packet sizes.
//!
//! [`open`] reads the 44-byte header, the audio track headers and the frame index, and builds a
//! table of [`Frame`]s. A frame holds one packet per audio track and then one video packet;
//! [`BinkFile::read_frame`] reads a frame and [`split_frame`] separates its packets.
//!
//! Every size and count comes from the file and is treated as hostile: the frame table is bounded
//! by the number of index entries the file could hold, and every frame lies inside the file. A
//! truncated file opens with the frames that are complete.
//!
//! No dependencies beyond `std`, no `unsafe`, builds for `wasm32-unknown-unknown`.

#![warn(missing_docs)]

#[cfg(test)]
mod tests;

pub use crate::source::{ByteSource, ReadSeekSource};

use std::io;

/// Size of the main header in bytes.
const HEADER_LEN: usize = 44;
/// The most audio tracks a file may declare.
const MAX_AUDIO_TRACKS: u32 = 256;
/// The largest width or height a file may declare.
const MAX_DIMENSION: u32 = 32767;
/// Index entries read at a time: 64 KiB.
const INDEX_PIECE_ENTRIES: u64 = 16384;

/// Errors from reading a Bink file.
#[non_exhaustive]
#[derive(Debug)]
pub enum Error {
    /// Not a Bink file (no `BIK` signature).
    NotBink,
    /// A Bink 2 file (`KB2` signature), which this module does not read.
    Bink2,
    /// A read failed.
    Io(io::Error),
    /// Structurally invalid data.
    Invalid(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotBink => f.write_str("not a Bink file"),
            Error::Bink2 => f.write_str("Bink 2 files are not supported"),
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Invalid(e) => write!(f, "invalid Bink file: {e}"),
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

fn invalid<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(msg.into()))
}

/// Whether `head` starts like a Bink 1 file: `BIK` and a lower-case revision letter.
pub fn sniff(head: &[u8]) -> bool {
    head.get(..3) == Some(b"BIK") && head.get(3).is_some_and(u8::is_ascii_lowercase)
}

/// The main header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// The four signature bytes: `BIK` and the codec revision letter (`b`, `d`, `f`, `g`, `h`, `i`).
    pub signature: [u8; 4],
    /// The frame count the header declares.
    pub frames: u32,
    /// The size in bytes of the largest frame, as declared.
    pub largest_frame: u32,
    /// Picture width in pixels.
    pub width: u32,
    /// Picture height in pixels.
    pub height: u32,
    /// Frame rate numerator.
    pub fps_num: u32,
    /// Frame rate denominator.
    pub fps_den: u32,
    /// The video flags as stored; see [`Header::has_alpha`], [`Header::grayscale`] and
    /// [`Header::scaling`].
    pub video_flags: u32,
}

impl Header {
    /// The codec revision letter (`b'i'` for `BIKi`).
    pub fn revision(&self) -> u8 {
        self.signature[3]
    }

    /// Whether frames carry an alpha plane (flag bit 20).
    pub fn has_alpha(&self) -> bool {
        self.video_flags & (1 << 20) != 0
    }

    /// Whether the video is grayscale (flag bit 17).
    pub fn grayscale(&self) -> bool {
        self.video_flags & (1 << 17) != 0
    }

    /// The display scaling mode (flag bits 28 to 31): 1 height doubled, 2 height interlaced,
    /// 3 width doubled, 4 width and height doubled, 5 width and height interlaced.
    pub fn scaling(&self) -> u8 {
        (self.video_flags >> 28) as u8
    }
}

/// An audio track header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioTrack {
    /// Samples per second.
    pub sample_rate: u16,
    /// The track flags as stored; see [`AudioTrack::stereo`] and [`AudioTrack::dct`].
    pub flags: u16,
    /// The track ID.
    pub id: u32,
}

impl AudioTrack {
    /// Whether the track is stereo (flag bit 13).
    pub fn stereo(&self) -> bool {
        self.flags & (1 << 13) != 0
    }

    /// Whether the track uses the DCT form of Bink Audio (flag bit 12); otherwise the RDFT form.
    pub fn dct(&self) -> bool {
        self.flags & (1 << 12) != 0
    }

    /// Codec parameters for a decoder: the codec, sample rate and channel count.
    pub fn codec_params(&self) -> crate::CodecParams {
        let id = if self.dct() { crate::CodecId::BinkAudioDct } else { crate::CodecId::BinkAudioRdft };
        crate::CodecParams::new(id).with_sample_rate(u32::from(self.sample_rate)).with_channels(if self.stereo() { 2 } else { 1 })
    }
}

/// Where one frame lies in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    /// Absolute file offset of the frame.
    pub offset: u64,
    /// Size of the frame in bytes: its audio packets and its video packet.
    pub size: u32,
    /// Whether the index marks the frame as a key frame.
    pub key: bool,
}

/// An opened Bink file: header, audio tracks and frame table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinkFile {
    /// The main header.
    pub header: Header,
    /// The audio tracks, in file order.
    pub audio: Vec<AudioTrack>,
    /// The frames that lie completely inside the file, in order. Shorter than
    /// [`Header::frames`] when the file is truncated.
    pub frames: Vec<Frame>,
}

/// One audio track's packet in a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioPacket<'a> {
    /// The count stored before the packet: the size in bytes of the PCM the packet decodes to.
    pub decoded_len: u32,
    /// The Bink Audio packet.
    pub data: &'a [u8],
}

/// The packets of one frame, borrowed from the frame's bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FramePackets<'a> {
    /// One entry per audio track; `None` when the track has no audio in this frame.
    pub audio: Vec<Option<AudioPacket<'a>>>,
    /// The Bink Video packet.
    pub video: &'a [u8],
}

fn le32(b: &[u8], at: usize) -> u32 {
    match b.get(at..at + 4) {
        Some(&[a, b, c, d]) => u32::from_le_bytes([a, b, c, d]),
        _ => 0,
    }
}

fn le16(b: &[u8], at: usize) -> u16 {
    match b.get(at..at + 2) {
        Some(&[a, b]) => u16::from_le_bytes([a, b]),
        _ => 0,
    }
}

/// Reads the headers and the frame index of a Bink 1 file.
pub fn open<S: ByteSource + ?Sized>(src: &S) -> Result<BinkFile> {
    let len = src.len();
    let mut head = [0u8; HEADER_LEN];
    if len < HEADER_LEN as u64 {
        // long enough to tell the signature apart, too short to be a file
        let mut sig = [0u8; 4];
        if len >= 4 {
            src.read_at(0, &mut sig)?;
        }
        return match sig {
            _ if sig.starts_with(b"KB2") => Err(Error::Bink2),
            _ if sniff(&sig) => invalid("file shorter than the header"),
            _ => Err(Error::NotBink),
        };
    }
    src.read_at(0, &mut head)?;
    if head.starts_with(b"KB2") {
        return Err(Error::Bink2);
    }
    if !sniff(&head) {
        return Err(Error::NotBink);
    }
    let header = Header {
        signature: [head[0], head[1], head[2], head[3]],
        frames: le32(&head, 8),
        largest_frame: le32(&head, 12),
        width: le32(&head, 20),
        height: le32(&head, 24),
        fps_num: le32(&head, 28),
        fps_den: le32(&head, 32),
        video_flags: le32(&head, 36),
    };
    let tracks = le32(&head, 40);
    if header.width == 0 || header.height == 0 || header.width > MAX_DIMENSION || header.height > MAX_DIMENSION {
        return invalid(format!("picture size {}x{}", header.width, header.height));
    }
    if header.fps_num == 0 || header.fps_den == 0 {
        return invalid("zero frame rate");
    }
    if tracks > MAX_AUDIO_TRACKS {
        return invalid(format!("{tracks} audio tracks"));
    }
    let tracks = tracks as usize;

    // three tables of 4 bytes per track: (unknown, channels), (sample rate, flags), ID
    let mut audio_head = vec![0u8; tracks * 12];
    src.read_at(HEADER_LEN as u64, &mut audio_head).map_err(|_| Error::Invalid("audio track headers cut off".into()))?;
    let audio = (0..tracks)
        .map(|t| AudioTrack {
            sample_rate: le16(&audio_head, tracks * 4 + t * 4),
            flags: le16(&audio_head, tracks * 4 + t * 4 + 2),
            id: le32(&audio_head, tracks * 8 + t * 4),
        })
        .collect();

    // the index: one absolute offset per frame and one for the end of the last frame
    let index_at = (HEADER_LEN + tracks * 12) as u64;
    // `len` was taken before the reads above: a source that reports less than it serves (a file
    // still being written) must not make this wrap
    let Some(room) = len.checked_sub(index_at) else { return invalid("audio track headers cut off") };
    if header.frames == 0 {
        return invalid("no frames");
    }
    // as many entries as the file declares and could hold
    let mut left = (u64::from(header.frames) + 1).min(room / 4);
    let data_start = index_at + (u64::from(header.frames) + 1) * 4;
    // The index is read in pieces and each entry checked as it arrives, so a hostile frame
    // count costs one piece, not the index it claims. The table grows as entries prove valid.
    let mut frames = Vec::new();
    let mut at = index_at;
    let mut previous: Option<u32> = None;
    let mut piece = Vec::new();
    'index: while left > 0 {
        let entries = left.min(INDEX_PIECE_ENTRIES) as usize;
        piece.resize(entries * 4, 0);
        src.read_at(at, &mut piece)?;
        for bytes in piece.as_chunks::<4>().0 {
            let entry = u32::from_le_bytes(*bytes);
            if let Some(start) = previous {
                let (offset, end) = (u64::from(start & !1), u64::from(entry & !1));
                if end > len {
                    break 'index; // truncated file: keep the frames that are whole
                }
                if offset < data_start || end < offset {
                    return invalid(format!("frame {} at {offset}..{end}", frames.len()));
                }
                frames.push(Frame { offset, size: (end - offset) as u32, key: start & 1 != 0 });
            }
            previous = Some(entry);
        }
        at += (entries * 4) as u64;
        left -= entries as u64;
    }
    Ok(BinkFile { header, audio, frames })
}

impl BinkFile {
    /// Codec parameters of the video: [`CodecId::BinkVideo`], the picture size, the signature as
    /// `tag` (its fourth byte is the codec revision) and the video flags, little-endian, as
    /// `extradata`.
    ///
    /// [`CodecId::BinkVideo`]: crate::CodecId::BinkVideo
    pub fn video_params(&self) -> crate::CodecParams {
        crate::CodecParams::new(crate::CodecId::BinkVideo)
            .with_tag(self.header.signature)
            .with_size(self.header.width, self.header.height)
            .with_extradata(self.header.video_flags.to_le_bytes())
    }

    /// Reads frame `i`: its audio packets and its video packet, as stored. [`split_frame`]
    /// separates them.
    pub fn read_frame<S: ByteSource + ?Sized>(&self, src: &S, i: usize) -> Result<Vec<u8>> {
        let Some(f) = self.frames.get(i) else { return invalid(format!("no frame {i}")) };
        // `open` checked the frame against the length the source had then
        if f.offset.checked_add(u64::from(f.size)).is_none_or(|e| e > src.len()) {
            return invalid(format!("frame {i} past the end of the file"));
        }
        // A frame can be up to 4 GiB. Reserving it can fail, and on a 32-bit target a size past
        // `isize::MAX` cannot be asked for at all: that is an error, not a panic.
        let mut data = Vec::new();
        let reserved = usize::try_from(f.size).ok().and_then(|size| data.try_reserve_exact(size).ok().map(|()| size));
        let Some(size) = reserved else { return invalid(format!("frame {i} of {} bytes is too large to read", f.size)) };
        data.resize(size, 0);
        src.read_at(f.offset, &mut data)?;
        Ok(data)
    }

    /// Separates the packets of a frame read with [`BinkFile::read_frame`].
    pub fn split_frame<'a>(&self, frame: &'a [u8]) -> Result<FramePackets<'a>> {
        split_frame(frame, self.audio.len())
    }
}

/// Separates the packets of a frame of a file with `tracks` audio tracks.
///
/// Each track has a 32-bit length; when it is not zero, a 32-bit decoded size and the packet
/// follow, and the length covers both. The video packet is what remains.
pub fn split_frame(frame: &[u8], tracks: usize) -> Result<FramePackets<'_>> {
    let mut rest = frame;
    // a track takes at least its 4-byte length, which bounds the table
    let mut audio = Vec::with_capacity(tracks.min(frame.len() / 4));
    for t in 0..tracks {
        let Some((len, after)) = rest.split_first_chunk::<4>() else { return invalid(format!("audio track {t}: length cut off")) };
        let len = u32::from_le_bytes(*len) as usize;
        if len == 0 {
            audio.push(None);
            rest = after;
            continue;
        }
        let Some((packet, after)) = after.split_at_checked(len) else { return invalid(format!("audio track {t}: packet of {len} bytes cut off")) };
        let Some((decoded_len, data)) = packet.split_first_chunk::<4>() else { return invalid(format!("audio track {t}: packet of {len} bytes")) };
        audio.push(Some(AudioPacket { decoded_len: u32::from_le_bytes(*decoded_len), data }));
        rest = after;
    }
    Ok(FramePackets { audio, video: rest })
}
