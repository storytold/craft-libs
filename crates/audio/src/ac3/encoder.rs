//! Standards-derived AC-3 and E-AC-3 encoding (ETSI TS 102 366 V1.4.1).
//!
//! Input uses explicit WAVE speaker order. Each packet represents 1536 samples,
//! including all dependent substreams for a 7.1 programme. The encoder uses the
//! conventional transform/quantization tools; AHT and spectral extension are not used.
//! Dual mono uses programme 1, programme 2, then optional LFE rather than a
//! speaker mask. The 7.1 compatibility downmix averages each side/rear pair
//! (−6 dB per input) to retain headroom; dependent data restores the originals.
mod bitstream;
mod transform;

use crate::ChannelLayout;

/// The output bitstream family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    /// AC-3 (`bsid = 8`).
    Ac3,
    /// Enhanced AC-3 (`bsid = 16`).
    Eac3,
}

/// Explicit speaker topology. Dual mono is not stereo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CodingMode {
    /// Two independently controlled mono programmes.
    DualMono = 0,
    /// Centre.
    Mono = 1,
    /// Left and right.
    Stereo = 2,
    /// Left, centre and right.
    ThreeFront = 3,
    /// Left, right and mono surround.
    StereoSurround = 4,
    /// Left, centre, right and mono surround.
    ThreeFrontSurround = 5,
    /// Left, right and a surround pair.
    StereoTwoSurround = 6,
    /// Left, centre, right and a surround pair.
    ThreeFrontTwoSurround = 7,
    /// E-AC-3: independent 5.1 downmix plus dependent original side/rear channels.
    Surround71 = 8,
}

impl CodingMode {
    /// Full-bandwidth channels, excluding LFE.
    pub fn channels(self) -> usize {
        if self == Self::Surround71 { 7 } else { super::NFCHANS[self as usize] }
    }
}

/// Constant-rate encoding settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// AC-3 or E-AC-3.
    pub codec: Codec,
    /// PCM sampling frequency: 32000, 44100 or 48000 Hz.
    pub sample_rate: u32,
    /// Programme data rate, including any dependent substream, in kilobits/second.
    pub bitrate_kbps: u32,
    /// Speaker topology.
    pub mode: CodingMode,
    /// LFE channel present. Required for `Surround71`.
    pub lfe: bool,
}

/// Invalid configuration, PCM or lifecycle use.
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// The requested configuration cannot be encoded.
    #[error("invalid AC-3 encoder settings: {0}")]
    InvalidSettings(&'static str),
    /// PCM channel lengths/counts or values are invalid.
    #[error("invalid AC-3 encoder input: {0}")]
    InvalidInput(&'static str),
    /// No more samples can be submitted after finalization.
    #[error("AC-3 encoder is already finished")]
    Finished,
    /// The configured frame cannot hold mandatory syntax.
    #[error("AC-3 frame bit budget exhausted")]
    BitBudget,
}

impl Config {
    /// Check format limits and supported configurations before accepting PCM.
    pub fn validate(&self) -> Result<(), Error> {
        if !matches!(self.sample_rate, 32_000 | 44_100 | 48_000) {
            return Err(Error::InvalidSettings("sample rate must be 32000, 44100 or 48000 Hz"));
        }
        if self.mode == CodingMode::Surround71 && (self.codec != Codec::Eac3 || !self.lfe) {
            return Err(Error::InvalidSettings("7.1 requires E-AC-3 and LFE"));
        }
        match self.codec {
            Codec::Ac3 if !super::BITRATES.contains(&self.bitrate_kbps) => {
                return Err(Error::InvalidSettings("bit rate is not an AC-3 frame-size table entry"));
            }
            Codec::Eac3 if !(32..=1024).contains(&self.bitrate_kbps) => {
                return Err(Error::InvalidSettings("E-AC-3 programme bit rate must be 32..1024 kb/s"));
            }
            _ => {}
        }
        if self.codec == Codec::Eac3 {
            let words = (u64::from(self.bitrate_kbps) * 96_000).div_ceil(u64::from(self.sample_rate));
            let largest = if self.mode == CodingMode::Surround71 { words * 3 / 5 } else { words };
            if largest > 2048 {
                return Err(Error::InvalidSettings("E-AC-3 syncframe exceeds its 11-bit word count"));
            }
        }
        let minimum = if self.mode == CodingMode::Surround71 { 384 } else { 32 * self.mode.channels() as u32 };
        if self.bitrate_kbps < minimum {
            return Err(Error::InvalidSettings("bit rate is too low for the selected encoder channel topology"));
        }
        Ok(())
    }

    /// Number of PCM planes.
    pub fn channels(&self) -> usize {
        self.mode.channels() + usize::from(self.lfe)
    }

    /// Required planar PCM speaker order (ascending WAVE mask bits).
    pub fn channel_layout(&self) -> ChannelLayout {
        if self.mode == CodingMode::DualMono {
            return ChannelLayout::unordered(self.channels() as u16);
        }
        let mask = match self.mode {
            CodingMode::Mono => 0x4,
            CodingMode::Stereo => 0x3,
            CodingMode::ThreeFront => 0x7,
            CodingMode::StereoSurround => 0x103,
            CodingMode::ThreeFrontSurround => 0x107,
            CodingMode::StereoTwoSurround => 0x603,
            CodingMode::ThreeFrontTwoSurround => 0x607,
            CodingMode::Surround71 => 0x637,
            CodingMode::DualMono => 0,
        };
        ChannelLayout::from_mask(mask | if self.lfe { 8 } else { 0 })
    }
}

/// One complete timed access unit, not an individual dependent syncframe.
#[derive(Clone, Debug)]
pub struct Packet {
    /// Raw syncframes in independent/dependent order.
    pub data: Vec<u8>,
    /// Decoded samples per channel, including any encoder priming or padding.
    pub samples: u32,
}

/// Final packets and exact container trimming information.
#[derive(Clone, Debug)]
pub struct Finish {
    /// Access units emitted while draining transform overlap.
    pub packets: Vec<Packet>,
    /// Samples to trim at the beginning of the decoded stream.
    pub priming_samples: u32,
    /// Samples to trim at the end of the decoded stream.
    pub padding_samples: u32,
}

/// Streaming encoder with bounded transform history.
pub struct Encoder {
    config: Config,
    pending: Vec<Vec<f32>>,
    history: Vec<[f32; 256]>,
    frame_index: u64,
    input_samples: u64,
    finished: bool,
    lfe_filter: Option<(usize, transform::LowPass)>,
}

impl Encoder {
    /// Construct a validated encoder; no PCM is consumed on error.
    pub fn new(config: Config) -> Result<Self, Error> {
        config.validate()?;
        Ok(Self {
            pending: vec![Vec::new(); config.channels()],
            history: vec![[0.0; 256]; config.channels()],
            config,
            frame_index: 0,
            input_samples: 0,
            finished: false,
            lfe_filter: config.lfe.then(|| {
                let index = if config.mode == CodingMode::Mono {
                    1
                } else if matches!(
                    config.mode,
                    CodingMode::ThreeFront | CodingMode::ThreeFrontSurround | CodingMode::ThreeFrontTwoSurround | CodingMode::Surround71
                ) {
                    3
                } else {
                    2
                };
                (index, transform::LowPass::new(config.sample_rate))
            }),
        })
    }

    /// Required input speaker assignments.
    pub fn channel_layout(&self) -> ChannelLayout {
        self.config.channel_layout()
    }

    /// The analysis window introduces 256 decoded priming samples.
    pub fn priming_samples(&self) -> u32 {
        256
    }

    /// Payload of an ISO BMFF `dac3` or `dec3` configuration box, without its box header.
    pub fn codec_configuration(&self) -> Vec<u8> {
        bitstream::configuration(self.config)
    }

    /// Accept equally sized finite PCM planes in the documented WAVE order.
    /// Values outside ±1 are rejected rather than silently clipped. Input errors
    /// leave the encoder untouched. A frame-encoding error is terminal; discard
    /// the stream rather than retrying input whose consumption is then unspecified.
    pub fn push(&mut self, planes: &[&[f32]]) -> Result<Vec<Packet>, Error> {
        if self.finished {
            return Err(Error::Finished);
        }
        if planes.len() != self.config.channels() {
            return Err(Error::InvalidInput("channel count differs from configuration"));
        }
        let count = planes.first().map_or(0, |p| p.len());
        if planes.iter().any(|p| p.len() != count || p.iter().any(|v| !v.is_finite() || v.abs() > 1.0)) {
            return Err(Error::InvalidInput("PCM must be finite, normalized and have equal plane lengths"));
        }
        self.input_samples =
            self.input_samples.checked_add(count as u64).filter(|&n| n <= u64::MAX - 1791).ok_or(Error::InvalidInput("sample count overflow"))?;
        let mut packets = Vec::new();
        let mut offset = 0;
        while offset < count {
            let take = (1536 - self.pending[0].len()).min(count - offset);
            for (ch, (dst, src)) in self.pending.iter_mut().zip(planes).enumerate() {
                if let Some((lfe, filter)) = &mut self.lfe_filter
                    && ch == *lfe
                {
                    dst.extend(src[offset..offset + take].iter().map(|&value| filter.process(value)));
                } else {
                    dst.extend_from_slice(&src[offset..offset + take]);
                }
            }
            offset += take;
            if self.pending[0].len() == 1536 {
                packets.push(self.emit()?);
            }
        }
        Ok(packets)
    }

    /// Drain the last overlap and return exact priming/padding counts. Empty input emits no packets.
    pub fn finish(&mut self) -> Result<Finish, Error> {
        if self.finished {
            return Err(Error::Finished);
        }
        if self.input_samples == 0 {
            self.finished = true;
            return Ok(Finish { packets: Vec::new(), priming_samples: 0, padding_samples: 0 });
        }
        let padded = self.input_samples.checked_add(256).ok_or(Error::InvalidInput("sample count overflow"))?;
        let required = padded.div_ceil(1536);
        let decoded_samples = required.checked_mul(1536).ok_or(Error::InvalidInput("sample count overflow"))?;
        let mut packets = Vec::new();
        while self.frame_index < required {
            for (ch, p) in self.pending.iter_mut().enumerate() {
                if let Some((lfe, filter)) = &mut self.lfe_filter
                    && ch == *lfe
                {
                    while p.len() < 1536 {
                        p.push(filter.process(0.0));
                    }
                } else {
                    p.resize(1536, 0.0);
                }
            }
            packets.push(self.emit()?);
        }
        self.finished = true;
        Ok(Finish { packets, priming_samples: 256, padding_samples: (decoded_samples - padded) as u32 })
    }

    fn emit(&mut self) -> Result<Packet, Error> {
        let mut spectra = vec![Vec::with_capacity(6); self.config.channels()];
        let mut short = [false; 6];
        for (block, short_block) in short.iter_mut().enumerate() {
            let windows: Vec<[f32; 512]> = (0..self.config.channels())
                .map(|ch| {
                    let mut window = [0.0; 512];
                    if block == 0 {
                        window[..256].copy_from_slice(&self.history[ch]);
                    } else {
                        window[..256].copy_from_slice(&self.pending[ch][(block - 1) * 256..block * 256]);
                    }
                    window[256..].copy_from_slice(&self.pending[ch][block * 256..(block + 1) * 256]);
                    window
                })
                .collect();
            *short_block =
                windows.iter().enumerate().any(|(ch, w)| self.lfe_filter.as_ref().is_none_or(|(lfe, _)| ch != *lfe) && transform::has_transient(w));
            for (ch, window) in windows.iter().enumerate() {
                let full_bandwidth = self.lfe_filter.as_ref().is_none_or(|(lfe, _)| ch != *lfe);
                spectra[ch].push(transform::analysis(window, *short_block && full_bandwidth));
            }
        }
        let data = match bitstream::encode(self.config, self.frame_index, &spectra, &short) {
            Ok(data) => data,
            Err(error) => {
                self.finished = true;
                return Err(error);
            }
        };
        for (history, pending) in self.history.iter_mut().zip(&mut self.pending) {
            history.copy_from_slice(&pending[1280..1536]);
            pending.clear();
        }
        self.frame_index += 1;
        Ok(Packet { data, samples: 1536 })
    }
}

#[cfg(test)]
mod tests;
