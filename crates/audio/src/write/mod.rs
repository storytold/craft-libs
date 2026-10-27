//! Encoding: WAV/BWF/RF64 and AIFF/AIFF-C writers (`pcm-encode`) and the FLAC encoder
//! (`flac-encode`), whole-buffer and streaming.

#[cfg(feature = "pcm-encode")]
mod aiff;
#[cfg(feature = "flac-encode")]
mod flac;
mod pcm;
#[cfg(feature = "pcm-encode")]
mod wav;

use crate::{AudioBuffer, AudioError, ChannelLayout, EncodeFormat, EncodeOptions, Result};
use std::io::{Seek, Write};

/// Encode `buf` as WAV/BWF, AIFF/AIFF-C or FLAC. Integer depths clamp samples to [-1, 1]; float
/// keeps values above full scale. `opts.layout`, when set, must have `buf`'s channel count.
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
    // Checked for every format; only a WAV records it.
    #[cfg_attr(not(feature = "pcm-encode"), allow(unused_variables))]
    let mask = opts.layout.map(|l| wav_mask(&l, buf.channels.len())).transpose()?;
    match opts.format {
        #[cfg(feature = "pcm-encode")]
        EncodeFormat::Wav => wav::encode(buf, opts.bit_depth, opts.dither, opts.bwf.as_ref(), opts.force_rf64, mask),
        #[cfg(feature = "pcm-encode")]
        EncodeFormat::Aiff => aiff::encode(buf, opts.bit_depth, opts.dither),
        #[cfg(feature = "flac-encode")]
        EncodeFormat::Flac => flac::encode(buf, opts.bit_depth, opts.dither),
        #[allow(unreachable_patterns)]
        f => Err(not_built(f)),
    }
}

/// The error for a format whose writer is not built.
#[allow(dead_code)]
fn not_built(format: EncodeFormat) -> AudioError {
    let feature = if format == EncodeFormat::Flac { "flac-encode" } else { "pcm-encode" };
    AudioError::Unsupported(format!("writing {format:?} needs the `{feature}` feature"))
}

/// The file extensions of the formats [`encode`] can write (those whose features are on).
#[cfg(all(feature = "pcm-encode", feature = "flac-encode"))]
pub const ENCODE_EXTENSIONS: &[&str] = &["wav", "wave", "bwf", "aif", "aiff", "flac"];
/// The file extensions of the formats [`encode`] can write (those whose features are on).
#[cfg(all(feature = "pcm-encode", not(feature = "flac-encode")))]
pub const ENCODE_EXTENSIONS: &[&str] = &["wav", "wave", "bwf", "aif", "aiff"];
/// The file extensions of the formats [`encode`] can write (those whose features are on).
#[cfg(all(feature = "flac-encode", not(feature = "pcm-encode")))]
pub const ENCODE_EXTENSIONS: &[&str] = &["flac"];

/// The format [`encode`] writes for a file extension or format name (`wav`, `aiff`, `flac`,
/// case-insensitive, with or without the dot), or `None` when it cannot write it.
///
/// ```
/// # #[cfg(feature = "pcm-encode")] {
/// use craft_audio::{EncodeFormat, encode_format_for};
///
/// assert_eq!(encode_format_for(".AIFF"), Some(EncodeFormat::Aiff));
/// assert_eq!(encode_format_for("mp3"), None);
/// # }
/// ```
pub fn encode_format_for(name: &str) -> Option<EncodeFormat> {
    match name.trim_start_matches('.').to_ascii_lowercase().as_str() {
        #[cfg(feature = "pcm-encode")]
        "wav" | "wave" | "bwf" => Some(EncodeFormat::Wav),
        #[cfg(feature = "pcm-encode")]
        "aif" | "aiff" => Some(EncodeFormat::Aiff),
        #[cfg(feature = "flac-encode")]
        "flac" => Some(EncodeFormat::Flac),
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
fn failed_write_error() -> AudioError {
    AudioError::Encode("the stream encoder already failed on an earlier write; its output is incomplete".into())
}

/// Writes WAV/BWF (RF64 above 4 GiB), AIFF / AIFF-C `fl32` or FLAC block by block, producing
/// the same bytes [`encode`] would for the whole buffer: a file of any length never
/// has to fit in memory.
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
pub struct FileEncoder<W: Write + Seek>(Writer<W>);

impl<W: Write + Seek> std::fmt::Debug for FileEncoder<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let format = match &self.0 {
            #[cfg(feature = "pcm-encode")]
            Writer::Wav(_) => EncodeFormat::Wav,
            #[cfg(feature = "pcm-encode")]
            Writer::Aiff(_) => EncodeFormat::Aiff,
            #[cfg(feature = "flac-encode")]
            Writer::Flac(_) => EncodeFormat::Flac,
        };
        f.debug_struct("FileEncoder").field("format", &format).finish_non_exhaustive()
    }
}

enum Writer<W> {
    #[cfg(feature = "pcm-encode")]
    Wav(wav::Writer<W>),
    #[cfg(feature = "pcm-encode")]
    Aiff(aiff::Writer<W>),
    #[cfg(feature = "flac-encode")]
    Flac(flac::Writer<W>),
}

impl<W: Write + Seek> FileEncoder<W> {
    /// Starts a file of `channels` channels at `sample_rate` at the current position of `out`.
    /// `opts.layout`, when set, must have `channels` channels.
    pub fn new(out: W, opts: &EncodeOptions, sample_rate: u32, channels: u16) -> Result<Self> {
        let (depth, dither, channels) = (opts.bit_depth, opts.dither, usize::from(channels));
        #[cfg_attr(not(feature = "pcm-encode"), allow(unused_variables))]
        let mask = opts.layout.map(|l| wav_mask(&l, channels)).transpose()?;
        Ok(FileEncoder(match opts.format {
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
            EncodeFormat::Flac => Writer::Flac(flac::Writer::new(out, channels, sample_rate, depth, dither)?),
            #[allow(unreachable_patterns)]
            f => return Err(not_built(f)),
        }))
    }

    /// Fails, without writing anything, when `frames` more frames would not fit the format (an
    /// AIFF holds 4 GiB, a FLAC 2^36 frames).
    pub fn check_room(&self, frames: u64) -> Result<()> {
        match &self.0 {
            #[cfg(feature = "pcm-encode")]
            Writer::Wav(w) => w.check_room(frames),
            #[cfg(feature = "pcm-encode")]
            Writer::Aiff(w) => w.check_room(frames),
            #[cfg(feature = "flac-encode")]
            Writer::Flac(w) => w.check_room(frames),
        }
        .map(drop)
    }

    /// Appends the first `frames` frames of `block`, one slice per channel (`&[Vec<f32>]`,
    /// `&[&[f32]]`, …; missing samples are silence), unless they don't fit (see
    /// [`FileEncoder::check_room`]). Once a write has failed, the encoder is unusable: the sink
    /// may hold a prefix of the failed block, so every later `write` and `finish` fails instead of
    /// completing an inconsistent file.
    pub fn write<C: AsRef<[f32]>>(&mut self, block: &[C], frames: usize) -> Result<()> {
        match &mut self.0 {
            #[cfg(feature = "pcm-encode")]
            Writer::Wav(w) => w.write(block, frames),
            #[cfg(feature = "pcm-encode")]
            Writer::Aiff(w) => w.write(block, frames),
            #[cfg(feature = "flac-encode")]
            Writer::Flac(w) => w.write(block, frames),
        }
    }

    /// Completes the headers and hands back `out`, positioned at the end of the file.
    pub fn finish(self) -> Result<W> {
        match self.0 {
            #[cfg(feature = "pcm-encode")]
            Writer::Wav(w) => w.finish(),
            #[cfg(feature = "pcm-encode")]
            Writer::Aiff(w) => w.finish(),
            #[cfg(feature = "flac-encode")]
            Writer::Flac(w) => w.finish(),
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

// With only one of `pcm-encode` and `flac-encode`, the other's formats are refused, not silently
// written as something else.
#[cfg(all(test, not(all(feature = "pcm-encode", feature = "flac-encode"))))]
mod tests {
    use super::*;

    #[test]
    fn unbuilt_format_is_unsupported() {
        let (format, name) = if cfg!(feature = "pcm-encode") { (EncodeFormat::Flac, "flac") } else { (EncodeFormat::Wav, "wav") };
        let opts = EncodeOptions::new(format);
        let buf = AudioBuffer::new(48_000, 1, 16);
        assert!(matches!(encode(&buf, &opts), Err(AudioError::Unsupported(_))));
        let enc = FileEncoder::new(std::io::Cursor::new(Vec::new()), &opts, 48_000, 1);
        assert!(matches!(enc, Err(AudioError::Unsupported(_))));
        assert_eq!(encode_format_for(name), None);
    }
}
