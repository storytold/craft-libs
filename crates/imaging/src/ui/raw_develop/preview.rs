//! The preview's controls on the workspace canvas: the colour samplers' readouts and the clipping overlay. The
//! workspace owns the sampler tool, the points and the cache; these functions read the source for them.

use std::sync::Arc;

use craft_color_ui::ui::{ColorSamplers, SamplerCache, SamplerReadout, SamplerReadouts};
use craft_theme::Srgba;
use craft_ui::primitives::Revision;
use craft_ui::runtime::env::Label;

use super::source::RawDevelopSource;
use crate::scopes::ClippingMask;
use crate::ui::scopes::{ClippingMode, ClippingOverlay, ScopeReadout};

/// The readouts of `samplers` over the picture `before` or after, from `cache` while `revision`, the picture shown and the
/// points are unchanged. The samplers ask in RGB; with `lab` (Show Lab Color Readouts) each readout also carries its Lab value,
/// asked of the source again for the few points.
pub(super) fn sampler_readouts(
    samplers: &ColorSamplers,
    source: &dyn RawDevelopSource,
    before: bool,
    revision: u64,
    lab: bool,
    cache: &mut SamplerCache,
) -> Arc<Vec<SamplerReadout>> {
    let read = |at: [f64; 2]| match source.sample(at, before, false) {
        Some(ScopeReadout::Rgb([r, g, b])) => Srgba { r, g, b, a: 1.0 },
        _ => Srgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 },
    };
    let readouts = SamplerReadouts::new(samplers, read).revision(revision).document(before).get(cache);
    if !lab {
        return readouts;
    }
    let mut with_lab = (*readouts).clone();
    for readout in &mut with_lab {
        let at = readout.position.map(|v| v as f64);
        readout.lab = match source.sample(at, before, true) {
            Some(ScopeReadout::Lab([l, a, b])) => {
                Some([l.round().clamp(0.0, 100.0) as i16, a.round().clamp(-128.0, 127.0) as i16, b.round().clamp(-128.0, 127.0) as i16])
            }
            _ => None,
        };
    }
    Arc::new(with_lab)
}

/// The readout under `at` as the automation surface lists it: its `position`, `space` (`rgb` in 0..=255, or `lab`
/// when `lab` asks for the Lab value) and `values`; `null` where the source reads no pixel.
pub(super) fn readout_json(source: &dyn RawDevelopSource, at: [f64; 2], before: bool, lab: bool) -> serde_json::Value {
    let (space, values) = match source.sample(at, before, lab) {
        Some(ScopeReadout::Rgb(rgb)) => ("rgb", rgb.map(|v| v * 255.0)),
        Some(ScopeReadout::Lab(lab)) => ("lab", lab),
        _ => return serde_json::Value::Null,
    };
    serde_json::json!({ "position": at, "space": space, "values": values })
}

/// The source's clipping mask and its revision while a warning is on; nothing when `mode` shows none.
pub(super) fn clipping_state(source: &dyn RawDevelopSource, mode: ClippingMode) -> Option<(Arc<ClippingMask>, Revision)> {
    if matches!(mode, ClippingMode::None) { None } else { source.clipping() }
}

/// The overlay of `held` (from [`clipping_state`]) in `mode`. The overlay rebuilds its texture only when the revision,
/// the mode or the theme's inks change.
pub(super) fn clipping_overlay<'a>(label: impl Into<Label>, held: &'a (Arc<ClippingMask>, Revision), mode: ClippingMode) -> ClippingOverlay<'a> {
    ClippingOverlay::new(label, &held.0, held.1, mode)
}
