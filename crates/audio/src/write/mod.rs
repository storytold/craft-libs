//! Encoding: WAV/BWF/RF64 and AIFF/AIFF-C writers (`pcm-encode`), the FLAC encoder
//! (`flac-encode`) and the coded formats (AAC and ALAC in M4A, MP3, AC-3 and E-AC-3 streams, each
//! with its codec's feature), whole-buffer and streaming.

#[cfg(feature = "pcm-encode")]
mod aiff;
#[cfg(any(feature = "aac-encode", feature = "alac-encode", feature = "mp3-encode", feature = "ac3-encode", feature = "eac3-encode"))]
mod coded;
#[cfg(feature = "flac-encode")]
pub(crate) mod flac;
mod pcm;
#[cfg(feature = "vorbis-encode")]
mod vorbis;
#[cfg(feature = "vorbis-encode")]
pub use vorbis::{OGG_BITRATES, OggVorbisOptions, encode as encode_ogg, encode_ogg_vorbis, quality_for_bitrate};
#[cfg(feature = "pcm-encode")]
mod raw;
#[cfg(feature = "pcm-encode")]
mod wav;

use crate::{AudioBuffer, AudioError, ChannelLayout, EncodeFormat, EncodeOptions, Result};
#[cfg(feature = "pcm-encode")]
pub use raw::{pcm_aiff_file, pcm_aiff_header, pcm_wav_file, pcm_wav_header};
#[cfg(any(
    feature = "pcm-encode",
    feature = "flac-encode",
    feature = "aac-encode",
    feature = "alac-encode",
    feature = "mp3-encode",
    feature = "ac3-encode",
    feature = "eac3-encode"
))]
use std::io::{Seek, Write};

/// Encode `buf` as the file `opts.format` names. Integer depths clamp samples to [-1, 1]; float
/// keeps values above full scale. `opts.layout`, when set, must have `buf`'s channel count.
///
/// The lossy formats take `opts.rate_control`. AAC and ALAC are written as M4A, which records the
/// encoder's priming and the exact length, so the file decodes to `buf`'s frames. MP3, AC-3 and
/// E-AC-3 are raw streams, which have no place for that: they decode with the codec's delay in
/// front and padded to whole frames.
///
/// ```
/// # #[cfg(feature = "pcm-encode")] {
/// use craft_audio::{AudioBuffer, BitDepth, EncodeFormat, EncodeOptions, encode};
///
/// let buf = AudioBuffer::new(48_000, 2, 48_000); // one second of stereo silence
/// let opts = EncodeOptions::new(EncodeFormat::Wav).with_bit_depth(BitDepth::Int24).with_dither(true);
/// let wav: Vec<u8> = encode(&buf, &opts).unwrap();
/// assert_eq!(&wav[..4], b"RIFF");
/// # }
/// ```
pub fn encode(buf: &AudioBuffer, opts: &EncodeOptions) -> Result<Vec<u8>> {
    check_built(opts.format)?;
    // Checked for every format; WAV and FLAC record it.
    #[cfg_attr(not(any(feature = "pcm-encode", feature = "flac-encode")), allow(unused_variables))]
    let mask = opts.layout.map(|l| wav_mask(&l, buf.channels.len())).transpose()?;
    match opts.format {
        #[cfg(feature = "pcm-encode")]
        EncodeFormat::Wav => wav::encode(buf, opts.bit_depth, opts.dither, opts.bwf.as_ref(), opts.force_rf64, mask),
        #[cfg(feature = "pcm-encode")]
        EncodeFormat::Aiff => aiff::encode(buf, opts.bit_depth, opts.dither),
        #[cfg(feature = "flac-encode")]
        EncodeFormat::Flac => flac::encode(buf, opts.bit_depth, opts.dither, mask),
        #[cfg(any(feature = "aac-encode", feature = "alac-encode", feature = "mp3-encode", feature = "ac3-encode", feature = "eac3-encode"))]
        EncodeFormat::Aac | EncodeFormat::Alac | EncodeFormat::Mp3 | EncodeFormat::Ac3 | EncodeFormat::Eac3 => coded::encode(buf, opts),
        #[cfg(feature = "vorbis-encode")]
        EncodeFormat::Ogg => {
            let count = u16::try_from(buf.channels.len()).unwrap_or(u16::MAX);
            let layout = opts.layout.unwrap_or_else(|| ChannelLayout::default_for(count));
            if layout.channels() != count {
                return Err(AudioError::Unsupported(format!("a {}-channel layout for {count} channels", layout.channels())));
            }
            let map = vorbis::vorbis_map(&layout)?;
            let quality = match opts.rate_control {
                None => vorbis::quality_for_bitrate(192, buf.channels.len()),
                Some(crate::RateControl::Bitrate(bps)) => {
                    vorbis::quality_for_bitrate((bps / 1000).clamp(OGG_BITRATES.0, OGG_BITRATES.1), buf.channels.len())
                }
                Some(crate::RateControl::Quality(q)) if q.is_nan() => return Err(AudioError::Encode("the Vorbis quality is NaN".into())),
                Some(crate::RateControl::Quality(q)) => -1.0 + 11.0 * q.clamp(0.0, 1.0),
            };
            let planar: Vec<&[f32]> = map.iter().filter_map(|&i| buf.channels.get(i).map(Vec::as_slice)).collect();
            vorbis::encode_planar(buf.sample_rate, &planar, &OggVorbisOptions::new(quality))
        }
        #[allow(unreachable_patterns)]
        f => Err(AudioError::Unsupported(format!("writing {f:?}"))),
    }
}

/// Refuses a format whose writer is not built, before anything else about the request is checked.
fn check_built(format: EncodeFormat) -> Result<()> {
    if format.is_built() { Ok(()) } else { Err(AudioError::Unsupported(format!("writing {format:?} needs the `{}` feature", format.feature()))) }
}

/// Every extension [`ENCODE_EXTENSIONS`] can list, and how many of them the features built give.
const fn extensions() -> ([&'static str; 13], usize) {
    let mut all = [""; 13];
    let mut n = 0;
    macro_rules! add {
        ($built:expr, $($extension:literal),+) => {
            if $built {
                $(
                    all[n] = $extension;
                    n += 1;
                )+
            }
        };
    }
    add!(cfg!(feature = "pcm-encode"), "wav", "wave", "bwf", "aif", "aiff", "aifc");
    add!(cfg!(feature = "flac-encode"), "flac");
    add!(cfg!(any(feature = "aac-encode", feature = "alac-encode")), "m4a");
    add!(cfg!(feature = "mp3-encode"), "mp3");
    add!(cfg!(feature = "ac3-encode"), "ac3");
    add!(cfg!(feature = "eac3-encode"), "eac3", "ec3");
    add!(cfg!(feature = "vorbis-encode"), "ogg");
    (all, n)
}

const EXTENSIONS: ([&str; 13], usize) = extensions();

/// The file extensions of the formats [`encode`] can write (those whose features are on).
pub const ENCODE_EXTENSIONS: &[&str] = EXTENSIONS.0.split_at(EXTENSIONS.1).0;

/// The format [`encode`] writes for a file extension or format name (`wav`, `aiff`, `flac`, `m4a`,
/// `mp3`, `ac3`, `eac3`, case-insensitive, with or without the dot), or `None` when it cannot
/// write it. `aifc` is AIFF: float AIFF is written as AIFF-C. `m4a` is AAC when the AAC encoder
/// is built and ALAC otherwise; `alac` always names ALAC.
///
/// ```
/// # #[cfg(feature = "pcm-encode")] {
/// use craft_audio::{EncodeFormat, encode_format_for};
///
/// assert_eq!(encode_format_for(".AIFF"), Some(EncodeFormat::Aiff));
/// assert_eq!(encode_format_for("unknown"), None);
/// # }
/// ```
pub fn encode_format_for(name: &str) -> Option<EncodeFormat> {
    match name.trim_start_matches('.').to_ascii_lowercase().as_str() {
        #[cfg(feature = "pcm-encode")]
        "wav" | "wave" | "bwf" => Some(EncodeFormat::Wav),
        #[cfg(feature = "pcm-encode")]
        "aif" | "aiff" | "aifc" => Some(EncodeFormat::Aiff),
        #[cfg(feature = "flac-encode")]
        "flac" => Some(EncodeFormat::Flac),
        #[cfg(feature = "aac-encode")]
        "m4a" => Some(EncodeFormat::Aac),
        #[cfg(all(feature = "alac-encode", not(feature = "aac-encode")))]
        "m4a" => Some(EncodeFormat::Alac),
        #[cfg(feature = "alac-encode")]
        "alac" => Some(EncodeFormat::Alac),
        #[cfg(feature = "mp3-encode")]
        "mp3" => Some(EncodeFormat::Mp3),
        #[cfg(feature = "ac3-encode")]
        "ac3" => Some(EncodeFormat::Ac3),
        #[cfg(feature = "eac3-encode")]
        "eac3" | "ec3" => Some(EncodeFormat::Eac3),
        #[cfg(feature = "vorbis-encode")]
        "ogg" => Some(EncodeFormat::Ogg),
        _ => None,
    }
}

/// The WAV speaker mask for `layout` over `channels` channels: its mask, or 0 (no speaker
/// assignment) for an unordered layout. A layout for another channel count is an error.
fn wav_mask(layout: &ChannelLayout, channels: usize) -> Result<u32> {
    if usize::from(layout.channels()) != channels {
        return Err(AudioError::Encode(format!("a {}-channel layout for {channels} channels", layout.channels())));
    }
    Ok(layout.mask().unwrap_or(0))
}

/// The error a stream encoder returns for every call after one of its writes has failed: the
/// sink may already hold a prefix of the failed block, so the encoder's counters can no longer
/// describe the bytes on the sink, and later writes or `finish` must not claim success.
#[cfg(any(
    feature = "pcm-encode",
    feature = "flac-encode",
    feature = "aac-encode",
    feature = "alac-encode",
    feature = "mp3-encode",
    feature = "ac3-encode",
    feature = "eac3-encode"
))]
fn failed_write_error() -> AudioError {
    AudioError::Encode("the stream encoder already failed on an earlier write; its output is incomplete".into())
}

/// Writes a file of any [`EncodeFormat`] block by block, producing the same bytes [`encode`]
/// would for the whole buffer: a file of any length never has to fit in memory. The file starts
/// at the sink's position, and nothing may follow that position (a new file, or one truncated
/// there): the encoder cannot truncate a `Write + Seek`, so the bytes of a longer file already
/// there would stay after the new one.
///
/// ```
/// # #[cfg(feature = "flac-encode")] {
/// use craft_audio::{EncodeFormat, EncodeOptions, FileEncoder};
/// use std::io::Cursor;
///
/// let opts = EncodeOptions::new(EncodeFormat::Flac);
/// let mut enc = FileEncoder::new(Cursor::new(Vec::new()), &opts, 48_000, 2).unwrap();
/// let block = [vec![0.25f32; 4096], vec![-0.25f32; 4096]]; // one Vec (or slice) per channel
/// for _ in 0..10 {
///     enc.write(&block, 4096).unwrap();
/// }
/// let flac = enc.finish().unwrap().into_inner();
/// assert_eq!(&flac[..4], b"fLaC");
/// # }
/// ```
#[cfg(any(
    feature = "pcm-encode",
    feature = "flac-encode",
    feature = "aac-encode",
    feature = "alac-encode",
    feature = "mp3-encode",
    feature = "ac3-encode",
    feature = "eac3-encode"
))]
pub struct FileEncoder<W: Write + Seek> {
    writer: Writer<W>,
    format: EncodeFormat,
    channels: usize,
}

#[cfg(any(
    feature = "pcm-encode",
    feature = "flac-encode",
    feature = "aac-encode",
    feature = "alac-encode",
    feature = "mp3-encode",
    feature = "ac3-encode",
    feature = "eac3-encode"
))]
impl<W: Write + Seek> std::fmt::Debug for FileEncoder<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileEncoder").field("format", &self.format).finish_non_exhaustive()
    }
}

// The writers differ in size with the features built; the coded one is boxed for its encoder.
#[allow(clippy::large_enum_variant)]
#[cfg(any(
    feature = "pcm-encode",
    feature = "flac-encode",
    feature = "aac-encode",
    feature = "alac-encode",
    feature = "mp3-encode",
    feature = "ac3-encode",
    feature = "eac3-encode"
))]
enum Writer<W: Write + Seek> {
    #[cfg(feature = "pcm-encode")]
    Wav(wav::Writer<W>),
    #[cfg(feature = "pcm-encode")]
    Aiff(aiff::Writer<W>),
    #[cfg(feature = "flac-encode")]
    Flac(flac::Writer<W>),
    #[cfg(any(feature = "aac-encode", feature = "alac-encode", feature = "mp3-encode", feature = "ac3-encode", feature = "eac3-encode"))]
    Coded(Box<coded::Writer<W>>),
}

#[cfg(any(
    feature = "pcm-encode",
    feature = "flac-encode",
    feature = "aac-encode",
    feature = "alac-encode",
    feature = "mp3-encode",
    feature = "ac3-encode",
    feature = "eac3-encode"
))]
impl<W: Write + Seek> FileEncoder<W> {
    /// Starts a file of `channels` channels at `sample_rate` at the current position of `out`.
    /// `opts.layout`, when set, must have `channels` channels.
    pub fn new(out: W, opts: &EncodeOptions, sample_rate: u32, channels: u16) -> Result<Self> {
        check_built(opts.format)?;
        let channels = usize::from(channels);
        #[cfg_attr(not(any(feature = "pcm-encode", feature = "flac-encode")), allow(unused_variables))]
        let (depth, dither) = (opts.bit_depth, opts.dither);
        #[cfg_attr(not(any(feature = "pcm-encode", feature = "flac-encode")), allow(unused_variables))]
        let mask = opts.layout.map(|l| wav_mask(&l, channels)).transpose()?;
        let writer = match opts.format {
            #[cfg(feature = "pcm-encode")]
            EncodeFormat::Wav => Writer::Wav(wav::Writer::new(
                out,
                channels,
                sample_rate,
                depth,
                dither,
                wav::HeaderExtras { bwf: opts.bwf.as_ref(), mask, force_rf64: opts.force_rf64 },
            )?),
            #[cfg(feature = "pcm-encode")]
            EncodeFormat::Aiff => Writer::Aiff(aiff::Writer::new(out, channels, sample_rate, depth, dither)?),
            #[cfg(feature = "flac-encode")]
            EncodeFormat::Flac => Writer::Flac(flac::Writer::new(out, channels, sample_rate, depth, dither, mask)?),
            #[cfg(any(feature = "aac-encode", feature = "alac-encode", feature = "mp3-encode", feature = "ac3-encode", feature = "eac3-encode"))]
            EncodeFormat::Aac | EncodeFormat::Alac | EncodeFormat::Mp3 | EncodeFormat::Ac3 | EncodeFormat::Eac3 => {
                Writer::Coded(Box::new(coded::Writer::new(out, opts, sample_rate, channels)?))
            }
            #[allow(unreachable_patterns)]
            f => return Err(AudioError::Unsupported(format!("writing {f:?}"))),
        };
        Ok(FileEncoder { writer, format: opts.format, channels })
    }

    /// The constant bit rate the file is written at, in bits per second for all channels
    /// together: the rate asked for or the format's default, moved to one the stream
    /// can carry as [`RateControl::Bitrate`](crate::RateControl::Bitrate) describes. `None` for the lossless
    /// formats and for AAC at a constant quality.
    pub fn bitrate(&self) -> Option<u32> {
        match &self.writer {
            #[cfg(any(feature = "aac-encode", feature = "alac-encode", feature = "mp3-encode", feature = "ac3-encode", feature = "eac3-encode"))]
            Writer::Coded(w) => w.bitrate(),
            #[allow(unreachable_patterns)]
            _ => None,
        }
    }

    /// Fails, without writing anything, when `frames` more frames would not fit the format (an
    /// AIFF holds 4 GiB, a FLAC 2^36 - 1 frames). The coded formats have no limit a file reaches.
    pub fn check_room(&self, frames: u64) -> Result<()> {
        match &self.writer {
            #[cfg(feature = "pcm-encode")]
            Writer::Wav(w) => w.check_room(frames).map(drop),
            #[cfg(feature = "pcm-encode")]
            Writer::Aiff(w) => w.check_room(frames).map(drop),
            #[cfg(feature = "flac-encode")]
            Writer::Flac(w) => w.check_room(frames).map(drop),
            #[cfg(any(feature = "aac-encode", feature = "alac-encode", feature = "mp3-encode", feature = "ac3-encode", feature = "eac3-encode"))]
            Writer::Coded(_) => {
                let _ = frames;
                Ok(())
            }
        }
    }

    /// Appends the first `frames` frames of `block`, one slice per channel (`&[Vec<f32>]`,
    /// `&[&[f32]]`, …), unless they don't fit (see [`FileEncoder::check_room`]). `block` must
    /// have one slice per channel, each at least `frames` long; otherwise nothing is written and
    /// the error is [`AudioError::Encode`]. Once a write has failed for any other reason, the
    /// encoder is unusable: the sink may hold a prefix of the failed block, so every later `write`
    /// and `finish` fails instead of completing an inconsistent file.
    pub fn write<C: AsRef<[f32]>>(&mut self, block: &[C], frames: usize) -> Result<()> {
        if block.len() != self.channels {
            return Err(AudioError::Encode(format!("a block of {} channels for a {}-channel file", block.len(), self.channels)));
        }
        if let Some(short) = block.iter().map(|c| c.as_ref().len()).filter(|&n| n < frames).min() {
            return Err(AudioError::Encode(format!("{frames} frames asked for, but a channel holds {short}")));
        }
        match &mut self.writer {
            #[cfg(feature = "pcm-encode")]
            Writer::Wav(w) => w.write(block, frames),
            #[cfg(feature = "pcm-encode")]
            Writer::Aiff(w) => w.write(block, frames),
            #[cfg(feature = "flac-encode")]
            Writer::Flac(w) => w.write(block, frames),
            #[cfg(any(feature = "aac-encode", feature = "alac-encode", feature = "mp3-encode", feature = "ac3-encode", feature = "eac3-encode"))]
            Writer::Coded(w) => w.write(block, frames),
        }
    }

    /// Encodes what is still buffered, completes the headers and hands back `out`, positioned at
    /// the end of the file.
    pub fn finish(self) -> Result<W> {
        match self.writer {
            #[cfg(feature = "pcm-encode")]
            Writer::Wav(w) => w.finish(),
            #[cfg(feature = "pcm-encode")]
            Writer::Aiff(w) => w.finish(),
            #[cfg(feature = "flac-encode")]
            Writer::Flac(w) => w.finish(),
            #[cfg(any(feature = "aac-encode", feature = "alac-encode", feature = "mp3-encode", feature = "ac3-encode", feature = "eac3-encode"))]
            Writer::Coded(w) => w.finish(),
        }
    }
}

impl EncodeOptions {
    /// Whether a file of `channels` channels at `sample_rate` can be written with these options:
    /// `Ok`, or the error [`FileEncoder::new`] and [`encode`] would return, with nothing written.
    /// For choosing a format before a file is created; [`EncodeFormat::sample_rates`],
    /// [`EncodeFormat::max_channels`] and [`EncodeFormat::supports_layout`] say what a format takes.
    ///
    /// ```
    /// # #[cfg(feature = "mp3-encode")] {
    /// use craft_audio::{EncodeFormat, EncodeOptions};
    ///
    /// let mp3 = EncodeOptions::new(EncodeFormat::Mp3).with_bitrate(200_000);
    /// assert!(mp3.check(48_000, 2).is_ok());
    /// assert!(mp3.check(96_000, 2).is_err()); // not an MP3 sample rate
    /// assert_eq!(mp3.bitrate(48_000, 2).unwrap(), Some(192_000)); // the table rate below 200 kbit/s
    /// # }
    /// ```
    pub fn check(&self, sample_rate: u32, channels: u16) -> Result<()> {
        self.bitrate(sample_rate, channels).map(drop)
    }

    /// The constant bit rate such a file would be written at ([`FileEncoder::bitrate`]), or the
    /// error [`EncodeOptions::check`] returns.
    pub fn bitrate(&self, sample_rate: u32, channels: u16) -> Result<Option<u32>> {
        #[cfg(feature = "vorbis-encode")]
        if self.format == EncodeFormat::Ogg {
            // Vorbis is whole-buffer only and VBR has no constant output bitrate.
            encode(&AudioBuffer::new(sample_rate, channels, 0), self)?;
            return Ok(None);
        }
        // The encoder a file would get, on a sink that is thrown away: what it accepts and the
        // rate it settles on are then the writer's by construction.
        #[cfg(any(
            feature = "pcm-encode",
            feature = "flac-encode",
            feature = "aac-encode",
            feature = "alac-encode",
            feature = "mp3-encode",
            feature = "ac3-encode",
            feature = "eac3-encode"
        ))]
        {
            Ok(FileEncoder::new(std::io::Cursor::new(Vec::new()), self, sample_rate, channels)?.bitrate())
        }
        #[cfg(not(any(
            feature = "pcm-encode",
            feature = "flac-encode",
            feature = "aac-encode",
            feature = "alac-encode",
            feature = "mp3-encode",
            feature = "ac3-encode",
            feature = "eac3-encode"
        )))]
        {
            let _ = (sample_rate, channels);
            Err(AudioError::Unsupported("streaming encoders are not built".into()))
        }
    }
}

/// The header of a 32-bit float WAV holding `frames` frames (little-endian samples follow it). Its
/// length doesn't depend on `frames`, so a recorder can rewrite it in place as the file grows; it
/// switches to RF64 above 4 GiB.
///
/// ```
/// use craft_audio::wav_float_header;
///
/// // A recorder writes the header, appends interleaved f32 samples, then rewrites the header.
/// let mut file = wav_float_header(48_000, 2, 0).unwrap();
/// let samples = [0.25f32, -0.25, 0.5, -0.5];
/// file.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
/// let header = wav_float_header(48_000, 2, 2).unwrap();
/// file[..header.len()].copy_from_slice(&header);
/// ```
#[cfg(feature = "pcm-encode")]
pub fn wav_float_header(sample_rate: u32, channels: u16, frames: u64) -> Result<Vec<u8>> {
    if channels == 0 || sample_rate == 0 {
        return Err(AudioError::Encode("a WAV needs channels and a sample rate".into()));
    }
    wav::header(usize::from(channels), sample_rate, (32, true), None, None, frames, false)
}
