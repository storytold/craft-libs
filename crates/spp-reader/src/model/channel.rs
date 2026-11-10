//! The reference app's channels: [`ProjectChannel`] (one channel bit) and [`ChannelMask`] (the `channelTypes`
//! bitmask stored on effects, sources and blending entries).
//!
//! Bit meanings: 1, 2, 8, 16, 128, 8192 and 4194304 are Verified (texture-cache entries); the others are
//! Observed from the identifiers of procedural outputs that feed them (`specular` → 4, `glossiness` → 64, …).

use std::fmt;

/// One channel, identified by its bit in `channelTypes` / `DataChannel.type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ProjectChannel {
    BaseColor,
    Height,
    Specular,
    Opacity,
    Emissive,
    Glossiness,
    Roughness,
    AnisotropyAngle,
    Metallic,
    Normal,
    AmbientOcclusion,
    Diffuse,
    SpecularLevel,
    Scattering,
    /// A bit whose channel is not known yet.
    Bit(u64),
}

/// (bit, channel, short name) for every named channel.
/// Format data: the bits of the user channels (`user 0` … `user 7`) in a channel mask as read back (format 92 moves them
/// to bits 64-71; the reader maps them back). Measured: "What changed between formats".
const USER_CHANNEL_FIRST_BIT: u32 = 14;
const USER_CHANNEL_LAST_BIT: u32 = 21;

const NAMED: &[(u64, ProjectChannel, &str)] = &[
    (1, ProjectChannel::BaseColor, "base colour"),
    (2, ProjectChannel::Height, "height"),
    (4, ProjectChannel::Specular, "specular"),
    (8, ProjectChannel::Opacity, "opacity"),
    (16, ProjectChannel::Emissive, "emissive"),
    (64, ProjectChannel::Glossiness, "glossiness"),
    (128, ProjectChannel::Roughness, "roughness"),
    (512, ProjectChannel::AnisotropyAngle, "anisotropy angle"),
    (8192, ProjectChannel::Metallic, "metallic"),
    (4194304, ProjectChannel::Normal, "normal"),
    (8388608, ProjectChannel::AmbientOcclusion, "ambient occlusion"),
    (16777216, ProjectChannel::Diffuse, "diffuse"),
    (33554432, ProjectChannel::SpecularLevel, "specular level"),
    (134217728, ProjectChannel::Scattering, "scattering"),
];

impl ProjectChannel {
    /// The channel for a single bit (unnamed bits become [`ProjectChannel::Bit`]).
    pub fn from_bit(bit: u64) -> ProjectChannel {
        NAMED.iter().find(|(b, ..)| *b == bit).map_or(ProjectChannel::Bit(bit), |(_, c, _)| *c)
    }

    /// Whether the channel holds one value per texel (the reference app's default formats). Others hold colours or vectors.
    pub fn is_single_valued(self) -> bool {
        !matches!(
            self,
            ProjectChannel::BaseColor
                | ProjectChannel::Specular
                | ProjectChannel::Emissive
                | ProjectChannel::Normal
                | ProjectChannel::Diffuse
                | ProjectChannel::Scattering
        )
    }

    pub fn bit(self) -> u64 {
        match self {
            ProjectChannel::Bit(b) => b,
            named => NAMED.iter().find(|(_, c, _)| *c == named).map_or(0, |(b, ..)| *b),
        }
    }
}

impl fmt::Display for ProjectChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match NAMED.iter().find(|(_, c, _)| c == self) {
            Some((_, _, name)) => f.write_str(name),
            None => match self.bit() {
                // Bits 14-21 are the user channels (see `Scalar::channel_mask`).
                bit if (1 << USER_CHANNEL_FIRST_BIT..=1 << USER_CHANNEL_LAST_BIT).contains(&bit) && bit.is_power_of_two() => {
                    write!(f, "user {}", bit.trailing_zeros() - USER_CHANNEL_FIRST_BIT)
                }
                bit => write!(f, "channel bit {bit}"),
            },
        }
    }
}

/// A `channelTypes` value. Older files store "every channel" as `u32::MAX`, newer ones as `u64::MAX`;
/// 0 on mask effects means the (single, grey) mask channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ChannelMask(pub u64);

impl ChannelMask {
    /// Every channel bit of the low 32 is set (all channels).
    pub fn is_all(self) -> bool {
        self.0 as u32 == u32::MAX
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn contains(self, channel: ProjectChannel) -> bool {
        self.0 & channel.bit() != 0
    }

    /// The channels in the mask, lowest bit first.
    pub fn channels(self) -> impl Iterator<Item = ProjectChannel> {
        (0..64).map(|i| 1u64 << i).filter(move |b| self.0 & b != 0).map(ProjectChannel::from_bit)
    }
}

impl fmt::Display for ChannelMask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_all() {
            return f.write_str("all");
        }
        if self.is_empty() {
            return f.write_str("mask");
        }
        let low = self.0 as u32;
        if self.0 >> 32 == 0 && low.count_ones() >= 24 {
            let missing: Vec<String> = ChannelMask(u64::from(!low)).channels().map(|c| c.to_string()).collect();
            return write!(f, "all except {}", missing.join("+"));
        }
        let names: Vec<String> = self.channels().map(|c| c.to_string()).collect();
        f.write_str(&names.join("+"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_round_trip_and_masks_list_channels() {
        assert_eq!(ProjectChannel::from_bit(4194304), ProjectChannel::Normal);
        assert_eq!(ProjectChannel::Normal.bit(), 4194304);
        assert_eq!(ProjectChannel::from_bit(32), ProjectChannel::Bit(32));
        let m = ChannelMask(1 | 16777216);
        assert_eq!(m.to_string(), "base colour+diffuse");
        assert!(m.contains(ProjectChannel::Diffuse));
        assert!(ChannelMask(u32::MAX as u64).is_all());
        assert!(ChannelMask(u64::MAX).is_all());
        assert_eq!(ChannelMask(0).to_string(), "mask");
        assert_eq!(ChannelMask(u32::MAX as u64 & !2).to_string(), "all except height");
    }
}
