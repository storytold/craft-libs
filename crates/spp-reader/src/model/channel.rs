//! The reference app's channels: [`ProjectChannel`] (one channel bit) and [`ChannelMask`] (the `channelTypes`
//! bitmask stored on effects, sources and blending entries).
//!
//! Every named bit is Measured: a fresh project declared every channel type the reference app offers, one fill gave
//! each its own value, and the saved file's `DataChannel.type`, the fill's per-channel `channelTypes` and the exports
//! agreed for all of them. The compact layout stores the bit's index instead (see `Scalar::channel_bit`); bits 49-56
//! hold the channels only that layout has (its indices 14-21).

use std::fmt;

/// One channel, identified by its bit in `channelTypes` / `DataChannel.type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ProjectChannel {
    BaseColor,
    Height,
    Specular,
    Opacity,
    Emissive,
    Displacement,
    Glossiness,
    Roughness,
    AnisotropyLevel,
    AnisotropyAngle,
    Transmissive,
    Reflection,
    Ior,
    Metallic,
    Normal,
    AmbientOcclusion,
    Diffuse,
    SpecularLevel,
    BlendingMask,
    Scattering,
    SheenOpacity,
    SheenRoughness,
    SheenColor,
    CoatTangent,
    CoatOpacity,
    CoatColor,
    CoatRoughness,
    CoatSpecularLevel,
    CoatNormal,
    ScatteringColor,
    Translucency,
    SpecularEdgeColor,
    AbsorptionColor,
    BaseWeight,
    BaseDiffuseRoughness,
    SpecularWeight,
    TransmissionWeight,
    CoatRoughnessAnisotropy,
    ThinFilmWeight,
    Tangent,
    /// A bit whose channel is not known yet.
    Bit(u64),
}

/// Format data: the bits of the user channels (`user 0` … `user 7`) in a channel mask as read back (format 92 moves them
/// to bits 64-71; the reader maps them back). Measured: "What changed between formats".
const USER_CHANNEL_FIRST_BIT: u32 = 14;
const USER_CHANNEL_LAST_BIT: u32 = 21;

/// (bit, channel, short name) for every named channel. File-format data; the compact layout's index is the bit's
/// position, except for the last seven (indices 14-17 and 19-21, see the module header).
const NAMED: &[(u64, ProjectChannel, &str)] = &[
    (1, ProjectChannel::BaseColor, "base colour"),
    (1 << 1, ProjectChannel::Height, "height"),
    (1 << 2, ProjectChannel::Specular, "specular"),
    (1 << 3, ProjectChannel::Opacity, "opacity"),
    (1 << 4, ProjectChannel::Emissive, "emissive"),
    (1 << 5, ProjectChannel::Displacement, "displacement"),
    (1 << 6, ProjectChannel::Glossiness, "glossiness"),
    (1 << 7, ProjectChannel::Roughness, "roughness"),
    (1 << 8, ProjectChannel::AnisotropyLevel, "anisotropy level"),
    (1 << 9, ProjectChannel::AnisotropyAngle, "anisotropy angle"),
    (1 << 10, ProjectChannel::Transmissive, "transmissive"),
    (1 << 11, ProjectChannel::Reflection, "reflection"),
    (1 << 12, ProjectChannel::Ior, "index of refraction"),
    (1 << 13, ProjectChannel::Metallic, "metallic"),
    (1 << 22, ProjectChannel::Normal, "normal"),
    (1 << 23, ProjectChannel::AmbientOcclusion, "ambient occlusion"),
    (1 << 24, ProjectChannel::Diffuse, "diffuse"),
    (1 << 25, ProjectChannel::SpecularLevel, "specular level"),
    (1 << 26, ProjectChannel::BlendingMask, "blending mask"),
    (1 << 27, ProjectChannel::Scattering, "scattering"),
    (1 << 28, ProjectChannel::SheenOpacity, "sheen opacity"),
    (1 << 29, ProjectChannel::SheenRoughness, "sheen roughness"),
    (1 << 30, ProjectChannel::SheenColor, "sheen colour"),
    (1 << 33, ProjectChannel::CoatTangent, "coat tangent"),
    (1 << 40, ProjectChannel::CoatOpacity, "coat opacity"),
    (1 << 41, ProjectChannel::CoatColor, "coat colour"),
    (1 << 42, ProjectChannel::CoatRoughness, "coat roughness"),
    (1 << 43, ProjectChannel::CoatSpecularLevel, "coat specular level"),
    (1 << 44, ProjectChannel::CoatNormal, "coat normal"),
    (1 << 45, ProjectChannel::ScatteringColor, "scattering colour"),
    (1 << 46, ProjectChannel::Translucency, "translucency"),
    (1 << 47, ProjectChannel::SpecularEdgeColor, "specular edge colour"),
    (1 << 48, ProjectChannel::AbsorptionColor, "absorption colour"),
    (1 << 49, ProjectChannel::BaseWeight, "base weight"),
    (1 << 50, ProjectChannel::BaseDiffuseRoughness, "base diffuse roughness"),
    (1 << 51, ProjectChannel::SpecularWeight, "specular weight"),
    (1 << 52, ProjectChannel::TransmissionWeight, "transmission weight"),
    (1 << 54, ProjectChannel::CoatRoughnessAnisotropy, "coat roughness anisotropy"),
    (1 << 55, ProjectChannel::ThinFilmWeight, "thin film weight"),
    (1 << 56, ProjectChannel::Tangent, "tangent"),
];

impl ProjectChannel {
    /// The channel for a single bit (unnamed bits become [`ProjectChannel::Bit`]).
    pub fn from_bit(bit: u64) -> ProjectChannel {
        NAMED.iter().find(|(b, ..)| *b == bit).map_or(ProjectChannel::Bit(bit), |(_, c, _)| *c)
    }

    /// Whether the channel usually holds one value per texel: everything but colours and directions. Measured: how
    /// many components a channel keeps follows its declared format (a grey format shows the mean of a uniform value's
    /// three components, an RGB format all three), so this is the reading for channels in their usual formats.
    pub fn is_single_valued(self) -> bool {
        !self.is_colour() && !self.is_direction()
    }

    /// Whether the file stores the channel's uniform values as linear colours of an sRGB-encoded original. Measured:
    /// values given in sRGB come back linear in the file on exactly these channels, whatever their format.
    pub fn is_colour(self) -> bool {
        matches!(
            self,
            ProjectChannel::BaseColor
                | ProjectChannel::Specular
                | ProjectChannel::Emissive
                | ProjectChannel::Transmissive
                | ProjectChannel::Diffuse
                | ProjectChannel::SheenColor
                | ProjectChannel::CoatColor
                | ProjectChannel::ScatteringColor
                | ProjectChannel::SpecularEdgeColor
                | ProjectChannel::AbsorptionColor
        )
    }

    /// Tangent-space directions packed into 0..1 (`n · 0.5 + 0.5`); uncovered they show a flat (0.5, 0.5, 1).
    pub fn is_direction(self) -> bool {
        matches!(self, ProjectChannel::Normal | ProjectChannel::CoatNormal | ProjectChannel::Tangent | ProjectChannel::CoatTangent)
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
        assert_eq!(ProjectChannel::from_bit(32), ProjectChannel::Displacement);
        assert_eq!(ProjectChannel::from_bit(1 << 53), ProjectChannel::Bit(1 << 53));
        let m = ChannelMask(1 | 16777216);
        assert_eq!(m.to_string(), "base colour+diffuse");
        assert!(m.contains(ProjectChannel::Diffuse));
        assert!(ChannelMask(u32::MAX as u64).is_all());
        assert!(ChannelMask(u64::MAX).is_all());
        assert_eq!(ChannelMask(0).to_string(), "mask");
        assert_eq!(ChannelMask(u32::MAX as u64 & !2).to_string(), "all except height");
        assert_eq!(ProjectChannel::from_bit(1 << 52), ProjectChannel::TransmissionWeight);
        assert_eq!(ProjectChannel::from_bit(1 << 14).to_string(), "user 0");
        assert!(ProjectChannel::SheenColor.is_colour() && !ProjectChannel::SheenColor.is_single_valued());
        assert!(ProjectChannel::Scattering.is_single_valued() && !ProjectChannel::Normal.is_single_valued());
        let bits: std::collections::BTreeSet<u64> = NAMED.iter().map(|(b, ..)| *b).collect();
        assert_eq!(bits.len(), NAMED.len(), "every named bit is distinct");
        assert!(bits.iter().all(|b| b.is_power_of_two() && !(1 << USER_CHANNEL_FIRST_BIT..=1 << USER_CHANNEL_LAST_BIT).contains(b)));
    }
}
