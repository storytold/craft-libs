//! Audio sample entry parameters and the audio codec configuration records.

use super::*;

/// Fields of audio sample entries (ISO v0 plus QuickTime sound description v1/v2 extensions).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct AudioParams {
    /// QuickTime sound description version (0, 1, 2). ISO files use 0.
    pub qt_version: u16,
    /// Number of audio channels.
    pub channels: u32,
    /// Bits per sample from the entry (16 for most compressed formats).
    pub sample_size: u16,
    /// Sample rate in Hz.
    pub sample_rate: f64,
    /// The QuickTime `compressionID` field, as read. Written back only for version 1 entries.
    pub compression_id: i16,
    /// v1 fields.
    pub samples_per_packet: u32,
    /// Bytes per compressed packet (v1).
    pub bytes_per_packet: u32,
    /// Bytes per uncompressed frame (v1).
    pub bytes_per_frame: u32,
    /// Bytes per uncompressed sample (v1).
    pub bytes_per_sample: u32,
    /// v2 fields.
    pub const_bits_per_channel: u32,
    /// Linear PCM format flags from a version 2 entry. Bit 1 is float, bit 2 big-endian and bit 4
    /// signed integer; float samples count as signed.
    pub lpcm_flags: u32,
    /// Constant bytes per packet, or 0 when variable.
    pub const_bytes_per_packet: u32,
    /// Constant frames per packet, or 0 when variable.
    pub const_frames_per_packet: u32,
    /// `enda` little-endian flag inside a QuickTime `wave` box.
    pub little_endian: Option<bool>,
}

/// AAC (MPEG-4 audio) configuration from `esds`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct AacConfig {
    /// AudioSpecificConfig bytes (DecoderSpecificInfo).
    pub asc: Vec<u8>,
    /// Audio object type (2 = AAC-LC, 5 = SBR/HE-AAC, 29 = PS …) as signaled first in the ASC.
    pub object_type: u8,
    /// Sampling rate signaled by the ASC (core rate; 0 if the ASC is missing).
    pub sample_rate: u32,
    /// Channel configuration from the ASC (0 = defined elsewhere).
    pub channel_config: u8,
    /// DecoderConfigDescriptor fields.
    pub max_bitrate: u32,
    /// Average bitrate in bits per second.
    pub avg_bitrate: u32,
    /// Decoding buffer size in bytes.
    pub buffer_size: u32,
}

const AAC_RATES: [u32; 13] = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];

impl AacConfig {
    /// Build from an AudioSpecificConfig, decoding its header fields.
    pub fn from_asc(asc: Vec<u8>) -> Self {
        let (object_type, sample_rate, channel_config) = parse_asc(&asc).unwrap_or((0, 0, 0));
        AacConfig { asc, object_type, sample_rate, channel_config, max_bitrate: 0, avg_bitrate: 0, buffer_size: 0 }
    }
    /// Channel count implied by the channel configuration (ISO/IEC 14496-3 Table 1.19 with Amd 4:
    /// 7 and 12 are 7.1 front / back, 11 is 6.1, 13 is 22.2, 14 is 7.1 with front heights), 0 if
    /// unspecified (a PCE) or reserved.
    pub fn channels(&self) -> u32 {
        match self.channel_config {
            1..=6 => self.channel_config as u32,
            11 => 7,
            7 | 12 | 14 => 8,
            13 => 24,
            _ => 0,
        }
    }
}

/// Decode (audio object type, sampling frequency, channel configuration) from an AudioSpecificConfig.
pub fn parse_asc(asc: &[u8]) -> Option<(u8, u32, u8)> {
    let mut r = BitReader::new(asc);
    let aot = |r: &mut BitReader| -> Option<u8> {
        let t = r.read_bits(5).ok()? as u8;
        if t == 31 { Some(32 + r.read_bits(6).ok()? as u8) } else { Some(t) }
    };
    let rate = |r: &mut BitReader| -> Option<u32> {
        let i = r.read_bits(4).ok()? as usize;
        if i == 15 { r.read_bits(24).ok() } else { AAC_RATES.get(i).copied() }
    };
    let t = aot(&mut r)?;
    let sr = rate(&mut r)?;
    let ch = r.read_bits(4).ok()? as u8;
    Some((t, sr, ch))
}

/// Uncompressed PCM description.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct PcmConfig {
    /// Bits per sample (8, 16, 24, 32 or 64).
    pub bits: u16,
    /// True for floating-point samples.
    pub float: bool,
    /// True for big-endian sample bytes.
    pub big_endian: bool,
    /// Signed integer samples (8-bit `raw ` is unsigned).
    pub signed: bool,
    /// Number of channels.
    pub channels: u32,
    /// Sample rate in Hz.
    pub sample_rate: f64,
}

impl PcmConfig {
    /// Bytes per interleaved audio frame (all channels).
    /// Saturates at `u32::MAX` for a hostile channel count or sample size.
    pub fn bytes_per_frame(&self) -> u32 {
        (self.bits as u32).div_ceil(8).saturating_mul(self.channels.max(1))
    }
}

/// `dOps` — Opus specific box.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct OpusConfig {
    /// Number of output channels.
    pub output_channels: u8,
    /// The `PreSkip` field, as read.
    pub pre_skip: u16,
    /// Sample rate of the original input in Hz (informational).
    pub input_sample_rate: u32,
    /// Output gain in Q7.8 fixed point dB.
    pub output_gain: i16,
    /// Channel mapping family (0 means RTP mapping with 1 or 2 channels).
    pub channel_mapping_family: u8,
    /// StreamCount, CoupledCount and ChannelMapping bytes when family != 0.
    pub channel_mapping: Vec<u8>,
}

impl OpusConfig {
    /// The Ogg Opus identification header (RFC 7845 §5.1) this `dOps` box describes: the same
    /// fields, little-endian, behind the `OpusHead` magic.
    pub fn opus_head(&self) -> Vec<u8> {
        let mut h = Vec::with_capacity(19 + self.channel_mapping.len());
        h.extend_from_slice(b"OpusHead");
        h.push(1);
        h.push(self.output_channels);
        h.extend_from_slice(&self.pre_skip.to_le_bytes());
        h.extend_from_slice(&self.input_sample_rate.to_le_bytes());
        h.extend_from_slice(&self.output_gain.to_le_bytes());
        h.push(self.channel_mapping_family);
        if self.channel_mapping_family != 0 {
            h.extend_from_slice(&self.channel_mapping);
        }
        h
    }

    /// Parse the `dOps` box payload (the bytes after the box header).
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut c = Cur::labeled(data, "dOps");
        let _version = c.u8()?;
        Ok(OpusConfig {
            output_channels: c.u8()?,
            pre_skip: c.u16()?,
            input_sample_rate: c.u32()?,
            output_gain: c.i16()?,
            channel_mapping_family: c.u8()?,
            channel_mapping: c.rest().to_vec(),
        })
    }
    /// Serializes to the payload of a `dOps` box (without the box header).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = BoxBuf::new();
        b.u8(0);
        b.u8(self.output_channels);
        b.u16(self.pre_skip);
        b.u32(self.input_sample_rate);
        b.i16(self.output_gain);
        b.u8(self.channel_mapping_family);
        b.bytes(&self.channel_mapping);
        b.buf
    }
}

/// `dfLa` — FLAC metadata blocks, with STREAMINFO decoded.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct FlacConfig {
    /// Raw metadata blocks (each with its 4-byte header) as stored in `dfLa`.
    pub metadata_blocks: Vec<u8>,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Number of channels.
    pub channels: u8,
    /// Bits per sample.
    pub bits_per_sample: u8,
    /// Total samples in the stream; 0 if unknown.
    pub total_samples: u64,
}

impl FlacConfig {
    /// Parses the payload of the `dfLa` full box (version and flags first), keeping the metadata blocks and decoding STREAMINFO when present. Fails with [`Error::Truncated`] when a field runs past the end of `data`.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut c = Cur::labeled(data, "dfLa");
        c.full_header()?;
        let blocks = c.rest().to_vec();
        let mut cfg = FlacConfig { metadata_blocks: blocks.clone(), ..Default::default() };
        let mut b = Cur::labeled(&blocks, "dfLa");
        let hdr = b.u32()?;
        if (hdr >> 24) & 0x7F == 0 {
            let si = b.bytes(((hdr & 0xFF_FFFF) as usize).min(34))?;
            if si.len() >= 18 {
                let mut r = BitReader::new(&si[10..18]);
                cfg.sample_rate = r.read_bits(20).unwrap_or(0);
                cfg.channels = r.read_bits(3).unwrap_or(0) as u8 + 1;
                cfg.bits_per_sample = r.read_bits(5).unwrap_or(0) as u8 + 1;
                cfg.total_samples = r.read_bits_u64(36).unwrap_or(0);
            }
        }
        Ok(cfg)
    }
}

/// QuickTime timecode sample description (`tmcd`) plus the start frame read from its first sample.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TimecodeConfig {
    /// Bit 0 drop frame, bit 1 24-hour max, bit 2 negative times OK, bit 3 counter.
    pub flags: u32,
    /// Time units per second for timecode sample durations.
    pub timescale: u32,
    /// Duration of one frame in `timescale` units.
    pub frame_duration: u32,
    /// Nominal frames per second (`numberOfFrames`), e.g. 30 for 29.97.
    pub frames_per_second: u8,
    /// Frame number of the first timecode sample (filled in by the demuxer).
    pub start_frame: Option<u32>,
    /// Source reel name (`name` box) if present.
    pub name: Option<String>,
}

impl TimecodeConfig {
    /// True when the `flags` drop-frame bit is set.
    pub fn drop_frame(&self) -> bool {
        self.flags & 1 != 0
    }
    /// Format a frame number as `HH:MM:SS:FF` (`;` before the frames when drop-frame).
    pub fn format_frame(&self, frame: u32) -> String {
        let fps = self.frames_per_second.max(1) as u64;
        let mut f = frame as u64;
        if self.drop_frame() && fps >= 30 {
            let drop = (fps / 30) * 2; // 2 frames per minute at 30, 4 at 60
            let per_min = fps * 60 - drop;
            let per_10 = per_min * 10 + drop;
            let d = f / per_10;
            let m = f % per_10;
            f += drop * 9 * d + if m > drop { drop * ((m - drop) / per_min) } else { 0 };
        }
        let ff = f % fps;
        let s = (f / fps) % 60;
        let mi = (f / fps / 60) % 60;
        let h = (f / fps / 3600) % 24;
        let sep = if self.drop_frame() { ';' } else { ':' };
        format!("{h:02}:{mi:02}:{s:02}{sep}{ff:02}")
    }
}
