//! [`ChannelBlending`]: one `DataBlending` entry (blend mode and opacity for a set of channels) and
//! [`ProjectBlendMode`], the reference app's `blendingMode` enum.
//!
//! Values outside the reference app's list stay [`ProjectBlendMode::Unknown`] with the stored number.

use super::channel::{ChannelMask, ProjectChannel};
use std::fmt;

/// The reference app's blend mode as stored (`DataBlending.blendingMode`). The numbers are the same in every format seen
/// (51 to 92): each one was read off a layer named through the reference app's public API, and the names are the API's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectBlendMode {
    /// 0: the channel is left out of the layer.
    Disable,
    /// 1
    Replace,
    /// 2: the default for colour-like channels.
    Normal,
    Darken,
    /// 4: the default of the ambient-occlusion channel and of the material's AO map.
    Multiply,
    ColorBurn,
    LinearBurn,
    Lighten,
    Screen,
    ColorDodge,
    /// 10: the default of the height channel.
    LinearDodge,
    Overlay,
    SoftLight,
    HardLight,
    VividLight,
    LinearLight,
    PinLight,
    Difference,
    Exclusion,
    Tint,
    Saturation,
    Color,
    Value,
    Divide,
    InverseDivide,
    /// 25: the default of effect groups; filter-only layers.
    Passthrough,
    /// 26: also `additionalNormalMapBlending`.
    NormalMapCombine,
    /// 27: the default of the normal channel.
    NormalMapDetail,
    NormalMapInverseDetail,
    Subtract,
    InverseSubtract,
    SignedAddition,
    /// Any other value.
    Unknown(u64),
}

/// Every named mode in stored order: a mode's number is its position.
const MODES: [(ProjectBlendMode, &str); 32] = [
    (ProjectBlendMode::Disable, "disable"),
    (ProjectBlendMode::Replace, "replace"),
    (ProjectBlendMode::Normal, "normal"),
    (ProjectBlendMode::Darken, "darken"),
    (ProjectBlendMode::Multiply, "multiply"),
    (ProjectBlendMode::ColorBurn, "color burn"),
    (ProjectBlendMode::LinearBurn, "linear burn"),
    (ProjectBlendMode::Lighten, "lighten"),
    (ProjectBlendMode::Screen, "screen"),
    (ProjectBlendMode::ColorDodge, "color dodge"),
    (ProjectBlendMode::LinearDodge, "linear dodge"),
    (ProjectBlendMode::Overlay, "overlay"),
    (ProjectBlendMode::SoftLight, "soft light"),
    (ProjectBlendMode::HardLight, "hard light"),
    (ProjectBlendMode::VividLight, "vivid light"),
    (ProjectBlendMode::LinearLight, "linear light"),
    (ProjectBlendMode::PinLight, "pin light"),
    (ProjectBlendMode::Difference, "difference"),
    (ProjectBlendMode::Exclusion, "exclusion"),
    (ProjectBlendMode::Tint, "tint"),
    (ProjectBlendMode::Saturation, "saturation"),
    (ProjectBlendMode::Color, "color"),
    (ProjectBlendMode::Value, "value"),
    (ProjectBlendMode::Divide, "divide"),
    (ProjectBlendMode::InverseDivide, "inverse divide"),
    (ProjectBlendMode::Passthrough, "passthrough"),
    (ProjectBlendMode::NormalMapCombine, "normal map combine"),
    (ProjectBlendMode::NormalMapDetail, "normal map detail"),
    (ProjectBlendMode::NormalMapInverseDetail, "normal map inverse detail"),
    (ProjectBlendMode::Subtract, "subtract"),
    (ProjectBlendMode::InverseSubtract, "inverse subtract"),
    (ProjectBlendMode::SignedAddition, "signed addition"),
];

impl ProjectBlendMode {
    pub fn from_raw(value: u64) -> ProjectBlendMode {
        usize::try_from(value).ok().and_then(|i| MODES.get(i)).map_or(ProjectBlendMode::Unknown(value), |(mode, _)| *mode)
    }

    /// The stored number.
    pub fn raw(self) -> u64 {
        match self {
            ProjectBlendMode::Unknown(n) => n,
            named => MODES.iter().position(|(m, _)| *m == named).unwrap_or(usize::MAX) as u64,
        }
    }
}

impl fmt::Display for ProjectBlendMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match MODES.iter().find(|(m, _)| m == self) {
            Some((_, name)) => f.write_str(name),
            None => write!(f, "mode {}", self.raw()),
        }
    }
}

/// One `DataBlending`: how the channels in `channels` blend onto what is below.
///
/// Format ≤ 59 stores one entry per channel bit (28–34 entries, defaults included); format ≥ 61 stores
/// only entries that differ from the defaults, often for several channels at once (Observed).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelBlending {
    pub channels: ChannelMask,
    pub mode: ProjectBlendMode,
    pub opacity: f32,
}

impl ChannelBlending {
    /// True for what the reference app stores when nothing was changed: full opacity and the channel's usual mode.
    pub fn is_default(&self) -> bool {
        if self.opacity != 1.0 {
            return false;
        }
        let usual = |c: ProjectChannel| match c {
            ProjectChannel::Height => self.mode == ProjectBlendMode::LinearDodge,
            ProjectChannel::Normal => self.mode == ProjectBlendMode::NormalMapDetail,
            ProjectChannel::AmbientOcclusion => self.mode == ProjectBlendMode::Multiply,
            _ => self.mode == ProjectBlendMode::Normal,
        };
        self.channels.channels().all(usual)
    }
}

/// The entry that covers `channel`, if any.
pub fn blending_for(entries: &[ChannelBlending], channel: ProjectChannel) -> Option<&ChannelBlending> {
    entries.iter().find(|b| b.channels.contains(channel))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_keep_their_numbers() {
        for raw in 0..40 {
            assert_eq!(ProjectBlendMode::from_raw(raw).raw(), raw);
        }
        assert_eq!(ProjectBlendMode::from_raw(25), ProjectBlendMode::Passthrough);
        assert_eq!(ProjectBlendMode::from_raw(11).to_string(), "overlay");
        assert_eq!(ProjectBlendMode::from_raw(32), ProjectBlendMode::Unknown(32));
        assert_eq!(ProjectBlendMode::from_raw(99).to_string(), "mode 99");
    }

    #[test]
    fn defaults_depend_on_the_channel() {
        let height = |mode| ChannelBlending { channels: ChannelMask(2), mode, opacity: 1.0 };
        assert!(height(ProjectBlendMode::LinearDodge).is_default());
        assert!(!height(ProjectBlendMode::Normal).is_default());
        let entries = [height(ProjectBlendMode::Normal)];
        assert!(blending_for(&entries, ProjectChannel::Height).is_some());
        assert!(blending_for(&entries, ProjectChannel::BaseColor).is_none());
    }
}
