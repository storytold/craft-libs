//! The data the main plot and the All Channels plots show.

use super::panel::{COLORS_CHANNEL, ChannelOverlay, HistogramSet, ScopeChannel};
use crate::ui::histogram::{ChannelColor, HistogramData};

/// The channel a key shows: the set's channel with that key, else the composite (the first channel);
/// [`COLORS_CHANNEL`] only when the set offers Colors, else the composite as well. `None` for an empty set.
pub(super) fn resolve_channel<'s>(set: &HistogramSet<'s>, key: &str) -> Option<Shown<'s>> {
    if key == COLORS_CHANNEL
        && let Some(overlay) = set.colors
        && !set.channels.is_empty()
    {
        return Some(Shown::Colors(overlay));
    }
    set.channels.iter().find(|channel| channel.key == key).or_else(|| set.channels.first()).copied().map(Shown::Channel)
}

/// What the main plot shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Shown<'s> {
    /// One channel.
    Channel(ScopeChannel<'s>),
    /// The colour channels overlaid.
    Colors(ChannelOverlay),
}

/// The data the plot draws for the Colors entry: `Light` is the RGB look of the set's three colour channels (fills
/// add); `Ink` is the inks look of its colour channels (fills multiply). Colors is always in colour.
fn colors_data(set: &HistogramSet<'_>, overlay: ChannelOverlay) -> HistogramData {
    let colors = || set.channels.iter().filter(|channel| channel.color_channel);
    match overlay {
        ChannelOverlay::Light => {
            let mut bins = colors().map(|channel| channel.bins);
            let mut next = || bins.next().unwrap_or(&[]);
            HistogramData::rgb([next(), next(), next()])
        }
        ChannelOverlay::Ink => {
            let inks: Vec<(&[u32], ChannelColor)> = colors().map(|channel| (channel.bins, channel.ink)).collect();
            HistogramData::channels(&inks)
        }
    }
}

/// What the main plot draws: the counted data (`shown` in its colour when `colored`, else neutral), the original
/// outlined over it, and the channel's name for the value text. `colors_name` names the Colors entry.
pub(super) struct PlotData {
    pub data: HistogramData,
    pub outline: Option<HistogramData>,
    pub name: String,
}

pub(super) fn plot_data(set: &HistogramSet<'_>, shown: Shown<'_>, colored: bool, colors_name: &str) -> PlotData {
    let original = |channel: Option<&ScopeChannel<'_>>| channel.map(|channel| HistogramData::tone(channel.bins, ChannelColor::Neutral));
    match shown {
        Shown::Channel(channel) => {
            let ink = if colored { channel.ink } else { ChannelColor::Neutral };
            PlotData {
                data: HistogramData::tone(channel.bins, ink),
                outline: original(set.original.and_then(|original| original.iter().find(|c| c.key == channel.key))),
                name: channel.label.to_owned(),
            }
        }
        Shown::Colors(overlay) => PlotData {
            data: colors_data(set, overlay),
            outline: original(set.original.and_then(|original| original.first())),
            name: colors_name.to_owned(),
        },
    }
}
