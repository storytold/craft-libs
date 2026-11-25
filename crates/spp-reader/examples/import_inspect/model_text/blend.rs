//! Blending entries as short text: only what differs from the channel defaults, entries with the same mode
//! and opacity merged ("base colour+diffuse multiply 50%"), so old files' one-entry-per-channel lists read
//! like new files' sparse ones.

use super::effects::short_list;
use craft_spp_reader::model::{ChannelBlending, ChannelMask, ProjectBlendMode};

/// Where the entries sit; decides what counts as a default.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum BlendContext {
    /// A layer, folder or content effect: per-channel defaults (normal, height linear dodge, …).
    Layer,
    /// An effect group: passthrough is its default (old files also store the layer defaults).
    Group,
    /// An effect in a mask stack: only the grey mask entry (channel mask 0) matters.
    Mask,
}

/// `None` if every entry is a default for its context.
pub(super) fn blending_text(entries: &[ChannelBlending], context: BlendContext) -> Option<String> {
    let mut merged: Vec<(ProjectBlendMode, f32, ChannelMask)> = Vec::new();
    for b in entries.iter().filter(|b| shown(b, context)) {
        match merged.iter_mut().find(|(m, o, _)| *m == b.mode && *o == b.opacity) {
            Some((_, _, mask)) => mask.0 |= b.channels.0,
            None => merged.push((b.mode, b.opacity, b.channels)),
        }
    }
    let texts: Vec<String> = merged
        .iter()
        .map(|(mode, opacity, channels)| {
            let opacity = if *opacity == 1.0 { String::new() } else { format!(" {:.0}%", opacity * 100.0) };
            format!("{channels} {mode}{opacity}")
        })
        .collect();
    (!texts.is_empty()).then(|| short_list(&texts, 6))
}

fn shown(b: &ChannelBlending, context: BlendContext) -> bool {
    match context {
        BlendContext::Layer => !b.is_default(),
        BlendContext::Group => !b.is_default() && (b.mode != ProjectBlendMode::Passthrough || b.opacity != 1.0),
        BlendContext::Mask => {
            let usual = matches!(b.mode, ProjectBlendMode::Normal | ProjectBlendMode::Passthrough);
            b.channels.is_empty() && (!usual || b.opacity != 1.0)
        }
    }
}
