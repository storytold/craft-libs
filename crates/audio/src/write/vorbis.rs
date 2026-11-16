//! Ogg Vorbis files (Vorbis I §A, RFC 5334) from the pure-Rust [`crate::vorbis::encoder`] and
//! craft-media's Ogg page writer: 1 to 8 channels in Vorbis channel order, a quality from -1 to
//! 10 (variable bit rate), and the tags in the comment header. Builds for every target.

use craft_media::ogg::{PageWriter, vorbis_comment};

use crate::vorbis::encoder::{DEFAULT_QUALITY, Encoder, EncoderConfig, Error as VorbisError};
use crate::{AudioBuffer, AudioError, ChannelLayout, Result};

/// The bit rates [`encode_ogg`](crate::encode_ogg) takes, in kbit/s. Vorbis is variable: the bit rate picks a
/// quality ([`quality_for_bitrate`]), and the file comes out near it for typical music.
pub const OGG_BITRATES: (u32, u32) = (64, 500);

/// The serial number of the stream when the options do not set one (`LCRF`): one stream per file,
/// so a constant keeps files byte-for-byte reproducible.
const SERIAL: u32 = 0x4C43_5246;

/// Samples per channel fed to the encoder at a time.
const CHUNK: usize = 4096;

/// What [`encode_ogg_vorbis`] writes besides the audio.
///
/// `#[non_exhaustive]`: build it with `OggVorbisOptions::default()` or
/// [`OggVorbisOptions::new`] and the `with_*` methods.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct OggVorbisOptions {
    /// -1 (smallest) to 10 (best); clamped, and NaN or infinite reads as
    /// [`DEFAULT_QUALITY`] (6).
    pub quality: f32,
    /// The comment header's vendor string.
    pub vendor: String,
    /// `NAME=value` comments (Vorbis I §5), in order. A name that is empty, or holds `=` or a byte
    /// outside 0x20–0x7D, is left out.
    pub tags: Vec<(String, String)>,
    /// The logical stream's serial number.
    pub serial: u32,
}

impl Default for OggVorbisOptions {
    fn default() -> Self {
        OggVorbisOptions { quality: DEFAULT_QUALITY, vendor: "craft-audio".into(), tags: Vec::new(), serial: SERIAL }
    }
}

impl OggVorbisOptions {
    /// The defaults at `quality`.
    pub fn new(quality: f32) -> Self {
        OggVorbisOptions { quality, ..Self::default() }
    }

    /// Sets [`quality`](Self::quality).
    pub fn with_quality(mut self, quality: f32) -> Self {
        self.quality = quality;
        self
    }

    /// Sets [`vendor`](Self::vendor).
    pub fn with_vendor(mut self, vendor: impl Into<String>) -> Self {
        self.vendor = vendor.into();
        self
    }

    /// Adds a comment ([`tags`](Self::tags)).
    pub fn with_tag(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.push((name.into(), value.into()));
        self
    }

    /// Sets [`serial`](Self::serial).
    pub fn with_serial(mut self, serial: u32) -> Self {
        self.serial = serial;
        self
    }
}

fn encode_error(e: VorbisError) -> AudioError {
    AudioError::Encode(format!("Vorbis: {e}"))
}

/// Encode `buf` as an Ogg Vorbis file with `opts`. The channels are in Vorbis channel order
/// (Vorbis I §4.3.9: L C R for three, L C R Ls Rs LFE for 5.1, L C R Ls Rs Rl Rr LFE for 7.1).
/// The last granule position is the buffer's length, so a decoder returns exactly its frames.
///
/// ```
/// use craft_audio::{AudioBuffer, OggVorbisOptions, encode_ogg_vorbis};
///
/// let buf = AudioBuffer::new(48_000, 6, 4_800); // 0.1 s of 5.1 silence
/// let opts = OggVorbisOptions::new(4.0).with_tag("TITLE", "Silence");
/// let ogg = encode_ogg_vorbis(&buf, &opts).unwrap();
/// assert!(ogg.starts_with(b"OggS"));
/// ```
pub fn encode_ogg_vorbis(buf: &AudioBuffer, opts: &OggVorbisOptions) -> Result<Vec<u8>> {
    let planar: Vec<&[f32]> = buf.channels.iter().map(Vec::as_slice).collect();
    encode_planar(buf.sample_rate, &planar, opts)
}

/// [`encode_ogg_vorbis`] over channels already in Vorbis order.
pub(crate) fn encode_planar(sample_rate: u32, planar: &[&[f32]], opts: &OggVorbisOptions) -> Result<Vec<u8>> {
    if !(1..=8).contains(&planar.len()) {
        return Err(AudioError::Unsupported(format!("Ogg Vorbis holds 1 to 8 channels, not {}", planar.len())));
    }
    let frames = planar.first().map_or(0, |c| c.len());
    if planar.iter().any(|c| c.len() != frames || c.iter().any(|s| !s.is_finite())) {
        return Err(AudioError::Encode("Vorbis samples must be finite and channels equally long".into()));
    }
    if sample_rate == 0 {
        return Err(AudioError::Encode("there is no audio to encode".into()));
    }
    let mut enc = Encoder::new(EncoderConfig::new(sample_rate, planar.len(), opts.quality)).map_err(encode_error)?;
    let mut pages = PageWriter::new(opts.serial);
    let tags: Vec<(&str, &str)> = opts.tags.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let mut out = pages.header(&enc.identification());
    out.extend(pages.header(&vorbis_comment(&opts.vendor, &tags)));
    out.extend(pages.header(&enc.setup()));
    // The first audio packet (which outputs nothing) gets a page of its own: a stream whose first
    // audio page is also its last would leave decoders to choose between start and end trimming
    // (Vorbis I §A.2), and ffmpeg then plays the padding.
    let mut first = true;
    let mut add = |packets: Vec<(Vec<u8>, i64)>, out: &mut Vec<u8>| {
        for (packet, granule) in packets {
            out.extend(pages.packet(&packet, granule));
            if std::mem::take(&mut first) {
                out.extend(pages.flush());
            }
        }
    };
    let mut at = 0;
    loop {
        let end = (at + CHUNK).min(frames);
        let chunk: Vec<&[f32]> = planar.iter().map(|c| c.get(at..end).unwrap_or_default()).collect();
        add(enc.encode(&chunk).map_err(encode_error)?, &mut out);
        at = end;
        if at >= frames {
            break;
        }
    }
    add(enc.finish().map_err(encode_error)?, &mut out);
    out.extend(pages.finish(None));
    Ok(out)
}

/// Nominal kbit/s per channel at each whole quality from -1 to 10: this encoder's rate on
/// music-like test signals at 48 kHz (harmonic chords with noise), averaged.
const KBPS_PER_CHANNEL: [f32; 12] = [18.0, 23.0, 31.0, 43.0, 58.0, 75.0, 89.0, 100.0, 111.0, 121.0, 130.0, 140.0];

/// The quality whose nominal bit rate is `kbps` kbit/s for `channels` channels (clamped to -1..=10).
/// Vorbis is variable: the bit rate of a file depends on the audio.
pub fn quality_for_bitrate(kbps: u32, channels: usize) -> f32 {
    let per = kbps as f32 / channels.max(1) as f32;
    let mut q = -1.0;
    for pair in KBPS_PER_CHANNEL.windows(2) {
        if let [lo, hi] = *pair
            && per <= hi
        {
            return q + ((per - lo) / (hi - lo)).clamp(0.0, 1.0);
        }
        q += 1.0;
    }
    10.0
}

/// Encode `buf` as an Ogg Vorbis stream near `kbps` kbit/s ([`OGG_BITRATES`], clamped): the
/// quality [`quality_for_bitrate`] gives, no tags. 1 to 8 channels in Vorbis channel order.
/// `Err` says why there is nothing.
pub fn encode(buf: &AudioBuffer, kbps: u32) -> Result<Vec<u8>> {
    let kbps = kbps.clamp(OGG_BITRATES.0, OGG_BITRATES.1);
    encode_ogg_vorbis(buf, &OggVorbisOptions::new(quality_for_bitrate(kbps, buf.channels.len())))
}

/// WAV-order speakers ([`crate::EncodeFormat::layout_masks`]) to Vorbis order, per channel count.
const VORBIS_ORDER: [&[usize]; 8] =
    [&[0], &[0, 1], &[0, 2, 1], &[0, 1, 2, 3], &[0, 2, 1, 3, 4], &[0, 2, 1, 4, 5, 3], &[0, 2, 1, 5, 6, 4, 3], &[0, 2, 1, 6, 7, 4, 5, 3]];

/// For each Vorbis channel, the channel of audio in `layout` it comes from.
pub(crate) fn vorbis_map(layout: &ChannelLayout) -> Result<Vec<usize>> {
    let n = layout.channels();
    let mask = crate::EncodeFormat::Ogg.layout_masks(n).first().copied();
    let order = usize::from(n).checked_sub(1).and_then(|i| VORBIS_ORDER.get(i)).copied();
    match (mask, order) {
        (Some(mask), Some(order)) => crate::layout::channel_map(mask, Some(order), layout)
            .ok_or_else(|| AudioError::Unsupported(format!("Vorbis has no layout of {n} channels with these speakers"))),
        _ => Err(AudioError::Unsupported(format!("Ogg Vorbis holds 1 to 8 channels, not {n}"))),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::{OGG_BITRATES, OggVorbisOptions, encode, encode_ogg_vorbis, quality_for_bitrate, vorbis_map};
    use crate::{AudioBuffer, AudioError, ChannelLayout};

    #[test]
    fn rejects_unsupported_or_malformed_channels_without_dropping_audio() {
        for channels in [0, 9] {
            assert!(matches!(encode(&AudioBuffer::new(48_000, channels, 16), 192), Err(AudioError::Unsupported(_))));
        }
        let ragged = AudioBuffer { sample_rate: 48_000, channels: vec![vec![0.0; 16], vec![0.0; 15]] };
        assert!(encode(&ragged, 192).is_err());
        assert!(encode(&AudioBuffer::new(0, 1, 16), 192).is_err());
        let invalid = AudioBuffer { sample_rate: 48_000, channels: vec![vec![f32::NAN]] };
        assert!(encode(&invalid, 192).is_err());
        let infinite = AudioBuffer { sample_rate: 48_000, channels: vec![vec![f32::INFINITY]] };
        assert!(encode(&infinite, 192).is_err());
    }

    #[test]
    fn generic_and_direct_encoders_write_one_to_eight_channels() {
        for channels in 1..=8 {
            let audio = AudioBuffer::new(48_000, channels, 512);
            let opts = crate::EncodeOptions::new(crate::EncodeFormat::Ogg);
            assert!(opts.check(48_000, channels).is_ok());
            assert_eq!(opts.bitrate(48_000, channels).unwrap(), None);
            let direct = encode(&audio, 192).unwrap();
            assert!(direct.starts_with(b"OggS"));
            // silence: the channel order changes nothing, so the generic path writes the same file
            let generic = crate::encode(&audio, &opts.with_bitrate(192_000)).unwrap();
            assert_eq!(direct, generic, "{channels} channels");
            #[cfg(feature = "decode")]
            {
                let (info, decoded) = crate::decode(&generic, Some("ogg")).unwrap();
                assert_eq!(info.channels, channels);
                assert_eq!(decoded.frames(), audio.frames());
            }
        }
    }

    #[test]
    fn the_quality_follows_the_bit_rate() {
        assert_eq!(quality_for_bitrate(0, 2), -1.0);
        assert_eq!(quality_for_bitrate(10_000, 2), 10.0);
        assert!((quality_for_bitrate(200, 2) - 6.0).abs() < 1e-6);
        assert!((quality_for_bitrate(600, 6) - 6.0).abs() < 1e-6);
        let mut last = -2.0;
        for kbps in OGG_BITRATES.0..=OGG_BITRATES.1 {
            let q = quality_for_bitrate(kbps, 2);
            assert!(q >= last && (-1.0..=10.0).contains(&q));
            last = q;
        }
        assert!(quality_for_bitrate(192, 0).is_finite());
    }

    #[test]
    fn layouts_map_to_vorbis_order() {
        // 5.1 in WAV order (L R C LFE Ls Rs) is written L C R Ls Rs LFE
        assert_eq!(vorbis_map(&ChannelLayout::SURROUND_5_1).unwrap(), [0, 2, 1, 4, 5, 3]);
        assert_eq!(vorbis_map(&ChannelLayout::SURROUND_5_1_SIDE).unwrap(), [0, 2, 1, 4, 5, 3]);
        assert_eq!(vorbis_map(&ChannelLayout::from_mask(0x63F)).unwrap(), [0, 2, 1, 6, 7, 4, 5, 3]);
        assert_eq!(vorbis_map(&ChannelLayout::from_mask(0x7)).unwrap(), [0, 2, 1]);
        assert_eq!(vorbis_map(&ChannelLayout::STEREO).unwrap(), [0, 1]);
        assert!(vorbis_map(&ChannelLayout::from_mask(0xC)).is_err());
        assert!(vorbis_map(&ChannelLayout::unordered(9)).is_err());
        assert!(vorbis_map(&ChannelLayout::unordered(0)).is_err());
    }

    #[test]
    fn options_build_and_tags_reach_the_comment_header() {
        let opts = OggVorbisOptions::default().with_quality(2.0).with_vendor("v").with_tag("TITLE", "t").with_tag("bad=name", "x").with_serial(9);
        assert_eq!((opts.quality, opts.vendor.as_str(), opts.serial, opts.tags.len()), (2.0, "v", 9, 2));
        let ogg = encode_ogg_vorbis(&AudioBuffer::new(44_100, 1, 100), &opts).unwrap();
        let file = craft_media::ogg::open(&ogg).unwrap();
        let s = &file.streams[0];
        assert_eq!((s.serial, s.saw_eos, s.headers.len()), (9, true, 3));
        assert_eq!(s.headers[1], craft_media::ogg::vorbis_comment("v", &[("TITLE", "t")]));
        assert_eq!(s.last_granule, Some(100));
    }
}
