//! [`ProxyPlan`]: when a zoomed-out preview renders on a reduced copy, and how reduced.

use craft_ui::canvas::PixelRect;

/// The zoom proxy of a live preview: zoomed out on a large document, the host previews on a copy
/// reduced by a power of two, built once per session and factor, so each change composites about
/// 1/k² of the pixels at no loss of detail on screen. Preferences › Performance › Low Resolution
/// Previews turns it off (PhotoCraft; on by default).
pub struct ProxyPlan;

impl ProxyPlan {
    /// Documents of at most this many pixels never use a proxy (PhotoCraft's `PROXY_MIN_PIXELS`).
    pub const MIN_PIXELS: u64 = 4_000_000;

    /// The largest reduction.
    pub const MAX_FACTOR: u32 = 64;

    /// The reduction for a view at `zoom` (physical screen pixels per document pixel) of a
    /// document of `document_pixels`: the largest power of two `k` ≤ [`MAX_FACTOR`](Self::MAX_FACTOR)
    /// with `2·k·zoom ≤ 1` (the proxy stays at least at screen resolution). 1 when `zoom` is
    /// above 0.5, not positive or NaN, when the document has at most
    /// [`MIN_PIXELS`](Self::MIN_PIXELS), or when `reduced_previews` is off. Zoom 1 gives 1, 0.5
    /// gives 2, 0.2 gives 4, 0.01 gives 64.
    pub fn factor(zoom: f32, document_pixels: u64, reduced_previews: bool) -> u32 {
        // pc: crates/ui-egui/src/dialogs/adjust_preview.rs:363 `proxy_factor` (`while k < 64 && (k * 2)
        // as f32 * zoom <= 1.0`), :344 `PROXY_MIN_PIXELS` (4_000_000), :403 (`!reduced_previews`) and
        // :412 (`area() <= PROXY_MIN_PIXELS`) give no proxy; crates/ui-egui/src/proxy.rs:21
        // `reduced_previews`.
        if !reduced_previews || document_pixels <= Self::MIN_PIXELS || zoom.is_nan() || zoom <= 0.0 {
            return 1;
        }
        let mut k = 1u32;
        while k < Self::MAX_FACTOR && (k * 2) as f32 * zoom <= 1.0 {
            k *= 2;
        }
        k
    }

    /// `region` in the pixels of a copy reduced by `k`: each edge divided by `k` rounding toward
    /// minus infinity, the far edges one pixel further (`x1 / k + 1`), so the reduced damage covers
    /// every pixel the full one touches. The host clips it to the proxy's size.
    pub fn scaled(region: PixelRect, k: u32) -> PixelRect {
        // pc: crates/ui-egui/src/dialogs/adjust_preview.rs:432 (`div_euclid(k)`, `+ 1` on x1 and y1).
        let k = i32::try_from(k.max(1)).unwrap_or(i32::MAX);
        PixelRect::new(region.x0.div_euclid(k), region.y0.div_euclid(k), region.x1.div_euclid(k) + 1, region.y1.div_euclid(k) + 1)
    }
}
