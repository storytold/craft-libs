//! Size limits and value checks for a dock layout.

use emath::Rect;

use super::split::SplitSize;

/// Limits are checked before cloning or recursively modifying an application layout.
pub const MAX_DEPTH: usize = 64;
/// Most panels a layout may hold. Larger layouts fail with [`DockError::LimitExceeded`](super::DockError::LimitExceeded).
pub const MAX_PANELS: usize = 4096;
pub(super) const MAX_NODES: usize = MAX_PANELS * 2;
const MAX_EXTENT: f32 = 1_000_000.0;

pub(super) fn valid_rect(rect: Rect) -> bool {
    let (position, size) = (rect.min, rect.size());
    [position.x, position.y, size.x, size.y].iter().all(|v| v.is_finite() && v.abs() <= MAX_EXTENT) && size.x > 0.0 && size.y > 0.0
}

pub(super) fn valid_width(width: f32) -> bool {
    width.is_finite() && width > 0.0 && width <= MAX_EXTENT
}

pub(super) fn valid_size(size: SplitSize) -> bool {
    match size {
        SplitSize::Ratio(r) => r.is_finite() && (0.0..=1.0).contains(&r),
        SplitSize::FixedFirst(v) | SplitSize::FixedSecond(v) => v.is_finite() && (0.0..=MAX_EXTENT).contains(&v),
    }
}

pub(super) fn valid_height(height: Option<f32>) -> bool {
    height.is_none_or(|v| v.is_finite() && (0.0..=MAX_EXTENT).contains(&v))
}

pub(super) fn valid_active(len: usize, active: usize) -> bool {
    active < len || (len == 0 && active == 0)
}
