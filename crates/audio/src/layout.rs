//! Channel layouts: which speaker each channel of a buffer feeds.
//!
//! A layout is a WAVE_FORMAT_EXTENSIBLE speaker mask (the same bits WAV files store in
//! `dwChannelMask`). Decoded audio is in WAV order: one channel per set bit, lowest bit first, so
//! L R C LFE, then back, side and height channels. A layout with no mask is *unordered*: the
//! channels are not assigned to speakers (Ambisonics, discrete multitrack, or a codec layout with
//! no WAV equivalent), and they come in the order the codec stores them.

/// A speaker position, with its WAVE_FORMAT_EXTENSIBLE mask bit.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Speaker {
    /// Front left (`SPEAKER_FRONT_LEFT`, 0x1).
    FrontLeft,
    /// Front right (`SPEAKER_FRONT_RIGHT`, 0x2).
    FrontRight,
    /// Front centre (`SPEAKER_FRONT_CENTER`, 0x4).
    FrontCenter,
    /// Low-frequency effects (`SPEAKER_LOW_FREQUENCY`, 0x8).
    Lfe,
    /// Back left (`SPEAKER_BACK_LEFT`, 0x10).
    BackLeft,
    /// Back right (`SPEAKER_BACK_RIGHT`, 0x20).
    BackRight,
    /// Front left of centre (`SPEAKER_FRONT_LEFT_OF_CENTER`, 0x40).
    FrontLeftOfCenter,
    /// Front right of centre (`SPEAKER_FRONT_RIGHT_OF_CENTER`, 0x80).
    FrontRightOfCenter,
    /// Back centre (`SPEAKER_BACK_CENTER`, 0x100).
    BackCenter,
    /// Side left (`SPEAKER_SIDE_LEFT`, 0x200).
    SideLeft,
    /// Side right (`SPEAKER_SIDE_RIGHT`, 0x400).
    SideRight,
    /// Top centre (`SPEAKER_TOP_CENTER`, 0x800).
    TopCenter,
    /// Top front left (`SPEAKER_TOP_FRONT_LEFT`, 0x1000).
    TopFrontLeft,
    /// Top front centre (`SPEAKER_TOP_FRONT_CENTER`, 0x2000).
    TopFrontCenter,
    /// Top front right (`SPEAKER_TOP_FRONT_RIGHT`, 0x4000).
    TopFrontRight,
    /// Top back left (`SPEAKER_TOP_BACK_LEFT`, 0x8000).
    TopBackLeft,
    /// Top back centre (`SPEAKER_TOP_BACK_CENTER`, 0x10000).
    TopBackCenter,
    /// Top back right (`SPEAKER_TOP_BACK_RIGHT`, 0x20000).
    TopBackRight,
}

impl Speaker {
    /// Every position, in mask-bit (and therefore WAV channel) order.
    pub const ALL: [Speaker; 18] = [
        Speaker::FrontLeft,
        Speaker::FrontRight,
        Speaker::FrontCenter,
        Speaker::Lfe,
        Speaker::BackLeft,
        Speaker::BackRight,
        Speaker::FrontLeftOfCenter,
        Speaker::FrontRightOfCenter,
        Speaker::BackCenter,
        Speaker::SideLeft,
        Speaker::SideRight,
        Speaker::TopCenter,
        Speaker::TopFrontLeft,
        Speaker::TopFrontCenter,
        Speaker::TopFrontRight,
        Speaker::TopBackLeft,
        Speaker::TopBackCenter,
        Speaker::TopBackRight,
    ];

    /// The WAVE_FORMAT_EXTENSIBLE mask bit (`SPEAKER_FRONT_LEFT` = 0x1, …).
    pub fn mask_bit(self) -> u32 {
        let index = Speaker::ALL.iter().position(|&s| s == self).unwrap_or(0);
        1 << index
    }

    /// A short label: `"L"`, `"R"`, `"C"`, `"LFE"`, `"Ls"`, …
    pub fn label(self) -> &'static str {
        match self {
            Speaker::FrontLeft => "L",
            Speaker::FrontRight => "R",
            Speaker::FrontCenter => "C",
            Speaker::Lfe => "LFE",
            Speaker::BackLeft => "Lb",
            Speaker::BackRight => "Rb",
            Speaker::FrontLeftOfCenter => "Lc",
            Speaker::FrontRightOfCenter => "Rc",
            Speaker::BackCenter => "Cb",
            Speaker::SideLeft => "Ls",
            Speaker::SideRight => "Rs",
            Speaker::TopCenter => "Tc",
            Speaker::TopFrontLeft => "Tfl",
            Speaker::TopFrontCenter => "Tfc",
            Speaker::TopFrontRight => "Tfr",
            Speaker::TopBackLeft => "Tbl",
            Speaker::TopBackCenter => "Tbc",
            Speaker::TopBackRight => "Tbr",
        }
    }
}

/// The bits [`Speaker::ALL`] covers.
const KNOWN_BITS: u32 = (1 << 18) - 1;

/// How the channels of a buffer map to speakers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(try_from = "RawLayout", into = "RawLayout"))]
pub struct ChannelLayout {
    channels: u16,
    /// 0 when unordered; otherwise exactly `channels` bits.
    mask: u32,
}

/// The serialized form of a [`ChannelLayout`], checked on the way in.
#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
struct RawLayout {
    channels: u16,
    mask: u32,
}

#[cfg(feature = "serde")]
impl From<ChannelLayout> for RawLayout {
    fn from(l: ChannelLayout) -> Self {
        RawLayout { channels: l.channels, mask: l.mask }
    }
}

#[cfg(feature = "serde")]
impl TryFrom<RawLayout> for ChannelLayout {
    type Error = String;

    fn try_from(r: RawLayout) -> Result<Self, String> {
        let ordered = r.mask & !KNOWN_BITS == 0 && r.mask.count_ones() == u32::from(r.channels);
        if r.mask == 0 || ordered {
            Ok(ChannelLayout { channels: r.channels, mask: r.mask })
        } else {
            Err(format!("speaker mask {:#x} does not describe {} channels", r.mask, r.channels))
        }
    }
}

impl ChannelLayout {
    /// C.
    pub const MONO: ChannelLayout = ChannelLayout { channels: 1, mask: 0x4 };
    /// L R.
    pub const STEREO: ChannelLayout = ChannelLayout { channels: 2, mask: 0x3 };
    /// L R C.
    pub const SURROUND_3_0: ChannelLayout = ChannelLayout { channels: 3, mask: 0x7 };
    /// L R Lb Rb.
    pub const QUAD: ChannelLayout = ChannelLayout { channels: 4, mask: 0x33 };
    /// L R C Lb Rb.
    pub const SURROUND_5_0: ChannelLayout = ChannelLayout { channels: 5, mask: 0x37 };
    /// L R C LFE Lb Rb.
    pub const SURROUND_5_1: ChannelLayout = ChannelLayout { channels: 6, mask: 0x3F };
    /// L R C LFE Ls Rs (Dolby's 5.1, with side surrounds).
    pub const SURROUND_5_1_SIDE: ChannelLayout = ChannelLayout { channels: 6, mask: 0x60F };
    /// L R C LFE Cb Ls Rs.
    pub const SURROUND_6_1: ChannelLayout = ChannelLayout { channels: 7, mask: 0x70F };
    /// L R C LFE Lb Rb Ls Rs.
    pub const SURROUND_7_1: ChannelLayout = ChannelLayout { channels: 8, mask: 0x63F };

    /// The layout of a speaker mask: one channel per set bit. Bits above the 18 standard positions
    /// make it unordered.
    pub fn from_mask(mask: u32) -> ChannelLayout {
        let channels = mask.count_ones() as u16;
        if mask & !KNOWN_BITS != 0 { ChannelLayout::unordered(usize::from(channels)) } else { ChannelLayout { channels, mask } }
    }

    /// `channels` channels described by `mask`, as WAV files store them: when the mask names a
    /// different number of speakers than there are channels, the channels are unordered.
    pub fn from_mask_and_channels(mask: u32, channels: usize) -> ChannelLayout {
        let layout = ChannelLayout::from_mask(mask);
        if mask != 0 && layout.is_ordered() && layout.channels() == channels { layout } else { ChannelLayout::unordered(channels) }
    }

    /// The usual layout for a channel count, as WAV readers assume when a file has no mask: mono,
    /// stereo, L R C, quad, 5.0, 5.1, 6.1 (with a back centre), 7.1 (backs and sides), 7.1.4.
    /// Other counts are unordered.
    pub fn default_for(channels: usize) -> ChannelLayout {
        let mask = match channels {
            1 => 0x4,
            2 => 0x3,
            3 => 0x7,
            4 => 0x33,
            5 => 0x37,
            6 => 0x3F,
            7 => 0x13F,
            8 => 0x63F,
            12 => 0x2_D63F,
            _ => 0,
        };
        if mask == 0 { ChannelLayout::unordered(channels) } else { ChannelLayout::from_mask(mask) }
    }

    /// `channels` channels with no speaker assignment.
    pub fn unordered(channels: usize) -> ChannelLayout {
        ChannelLayout { channels: u16::try_from(channels).unwrap_or(u16::MAX), mask: 0 }
    }

    /// The number of channels.
    pub fn channels(&self) -> usize {
        usize::from(self.channels)
    }

    /// The speaker mask, or `None` for an unordered layout.
    pub fn mask(&self) -> Option<u32> {
        (self.mask != 0).then_some(self.mask)
    }

    /// Whether the channels are assigned to speakers (and therefore in WAV order).
    pub fn is_ordered(&self) -> bool {
        self.mask != 0
    }

    /// The speaker of each channel in order; empty for an unordered layout.
    pub fn speakers(&self) -> Vec<Speaker> {
        Speaker::ALL.iter().copied().filter(|s| self.mask & s.mask_bit() != 0).collect()
    }

    /// The channel that feeds `speaker`, if the layout has it.
    pub fn index_of(&self, speaker: Speaker) -> Option<usize> {
        self.speakers().iter().position(|&s| s == speaker)
    }

    /// A conventional name: `"mono"`, `"stereo"`, `"5.1"`, `"7.1"`; `None` for other layouts.
    pub fn name(&self) -> Option<&'static str> {
        Some(match self.mask {
            0x4 => "mono",
            0x3 => "stereo",
            0xB => "2.1",
            0x7 => "3.0",
            0x33 => "quad",
            0x603 => "quad (side)",
            0x37 | 0x607 => "5.0",
            0x3F | 0x60F => "5.1",
            0x13F | 0x70F => "6.1",
            0x63F | 0xFF => "7.1",
            0x2_D63F => "7.1.4",
            _ => return None,
        })
    }
}

impl std::fmt::Display for ChannelLayout {
    /// The conventional name, or the channel count: `"5.1"`, `"10 channels"`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.name() {
            Some(name) => f.write_str(name),
            None if self.channels == 1 => f.write_str("1 channel"),
            None => write!(f, "{} channels", self.channels),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speakers_follow_the_mask_in_wav_order() {
        assert_eq!(
            ChannelLayout::SURROUND_5_1_SIDE.speakers(),
            [Speaker::FrontLeft, Speaker::FrontRight, Speaker::FrontCenter, Speaker::Lfe, Speaker::SideLeft, Speaker::SideRight]
        );
        assert_eq!(ChannelLayout::SURROUND_7_1.index_of(Speaker::SideLeft), Some(6));
        assert_eq!(Speaker::SideRight.mask_bit(), 0x400);
    }

    #[test]
    fn a_mask_that_does_not_match_the_channels_is_unordered() {
        assert_eq!(ChannelLayout::from_mask_and_channels(0x3F, 6), ChannelLayout::SURROUND_5_1);
        assert!(!ChannelLayout::from_mask_and_channels(0x3, 4).is_ordered());
        assert!(!ChannelLayout::from_mask_and_channels(0, 4).is_ordered());
        assert!(!ChannelLayout::from_mask(1 << 20).is_ordered());
        assert_eq!(ChannelLayout::unordered(4).channels(), 4);
        assert!(ChannelLayout::unordered(4).speakers().is_empty());
    }

    #[test]
    fn defaults_and_names() {
        assert_eq!(ChannelLayout::default_for(6), ChannelLayout::SURROUND_5_1);
        assert!(!ChannelLayout::default_for(9).is_ordered());
        assert_eq!(ChannelLayout::default_for(2).to_string(), "stereo");
        assert_eq!(ChannelLayout::SURROUND_5_1_SIDE.to_string(), "5.1");
        assert_eq!(ChannelLayout::unordered(10).to_string(), "10 channels");
    }
}
