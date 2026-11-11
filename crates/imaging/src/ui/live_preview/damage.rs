//! The damage of a key change and the covered area ([`LivePreview::switch`],
//! [`LivePreview::uncovered`]).

use craft_ui::canvas::PixelRect;

use super::key::PreviewKey;
use super::session::LivePreview;

impl LivePreview {
    /// The area to recomposite when the canvas switches from showing `from` to showing `to`, with
    /// `visible` the document pixels the view shows ([`PixelRect::visible`]).
    ///
    /// - Both keys [`owns`](Self::owns) and not both [`PreviewKey::NONE`]: to a preview, the
    ///   region's visible part (remembered as covered for `to`, so [`uncovered`](Self::uncovered)
    ///   catches up as the view moves); back to `NONE`, the whole region (and nothing stays
    ///   covered).
    /// - Either key foreign, both `NONE`, or no session: `None`, and the host recomposites
    ///   everything.
    ///
    /// The returned rect may be empty (a target out of view); the host then damages nothing.
    pub fn switch(&mut self, from: PreviewKey, to: PreviewKey, visible: PixelRect) -> Option<PixelRect> {
        // pc: crates/ui-egui/src/dialogs/adjust_preview.rs:292 `switch_region` (`ours(from) &&
        // ours(to) && (from != 0 || to != 0)`), :310 `switch_damage` (to 0: covered cleared, the
        // whole region; else `region ∩ visible`, covered = (to, that)); :525, :538 (the live lane's
        // twins).
        if !(self.owns(from) && self.owns(to) && (!from.is_none() || !to.is_none())) {
            return None;
        }
        if to.is_none() {
            self.covered = None;
            return Some(self.region);
        }
        let shown = self.region.intersect(&visible);
        self.covered = Some((to, shown));
        Some(shown)
    }

    /// The part of the region the canvas, showing `key`, has not been composited for within
    /// `visible` after the view moved: the region's visible part when it is not empty and not
    /// inside the covered rect; that part then becomes the covered rect. `None` when `key` is not
    /// the key last switched to, nothing new shows, or no session.
    pub fn uncovered(&mut self, key: PreviewKey, visible: PixelRect) -> Option<PixelRect> {
        // pc: crates/ui-egui/src/dialogs/adjust_preview.rs:323 `uncovered` (`want = region ∩ visible`;
        // none when empty or `covered.contains_rect(want)`; covered = want), :551 `live_uncovered`.
        self.session?;
        let (covered_key, covered) = self.covered?;
        if covered_key != key {
            return None;
        }
        let want = self.region.intersect(&visible);
        if want.is_empty() || contains(&covered, &want) {
            return None;
        }
        self.covered = Some((key, want));
        Some(want)
    }
}

/// Whether `outer` holds all of `inner`.
fn contains(outer: &PixelRect, inner: &PixelRect) -> bool {
    outer.x0 <= inner.x0 && outer.y0 <= inner.y0 && outer.x1 >= inner.x1 && outer.y1 >= inner.y1
}
