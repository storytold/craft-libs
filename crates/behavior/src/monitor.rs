//! The connected monitors, as every window placement reads them.

use emath::{Pos2, Rect, vec2};
use serde::{Deserialize, Serialize};

/// Identifies a monitor across launches: the name the platform reports and its size in pixels.
///
/// The size is part of the key because names repeat (`"Built-in Display"`) or change (`DP-1`
/// after a re-plug), and a monitor switched to another resolution is a different place to put a
/// window. A key that no longer matches falls back to the nearest connected monitor.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MonitorKey {
    /// The monitor name the platform reports.
    pub name: String,
    /// The monitor size in pixels, width then height.
    pub size_px: [u32; 2],
}

impl MonitorKey {
    /// A key for the monitor called `name` with size `size_px`.
    pub fn new(name: impl Into<String>, size_px: [u32; 2]) -> Self {
        Self { name: name.into(), size_px }
    }
}

/// A connected monitor, in physical pixels of the desktop's coordinate space (negative for a monitor left of or
/// above the primary). Points differ per monitor on a mixed-DPI desktop, so pixels are the one space every monitor
/// shares; [`points_at`](Self::points_at) gives the area in the points of one window.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Monitor {
    /// The monitor's identity.
    pub key: MonitorKey,
    /// Top-left corner in the desktop's pixel space.
    pub pos_px: [i32; 2],
    /// Width and height in pixels.
    pub size_px: [u32; 2],
    /// Physical pixels per point.
    pub scale: f32,
    /// The part not taken by the taskbar, Dock or menu bar: top-left corner and size in pixels.
    /// When set, it replaces [`Defaults::screen_room`](crate::placement::Defaults::screen_room) for this monitor.
    pub work_px: Option<([i32; 2], [u32; 2])>,
}

impl Monitor {
    /// A monitor at `pos_px` of `size_px` pixels with `scale`, usable in full.
    pub fn new(key: MonitorKey, pos_px: [i32; 2], size_px: [u32; 2], scale: f32) -> Self {
        Self { key, pos_px, size_px, scale, work_px: None }
    }

    /// Sets the usable area (see [`work_px`](Self::work_px)).
    pub fn with_work_px(mut self, pos_px: [i32; 2], size_px: [u32; 2]) -> Self {
        self.work_px = Some((pos_px, size_px));
        self
    }

    /// The monitor's area in pixels.
    pub fn rect_px(&self) -> Rect {
        let [x, y] = self.pos_px;
        let [width, height] = self.size_px;
        Rect::from_min_size(Pos2::new(x as f32, y as f32), vec2(width as f32, height as f32))
    }

    /// The monitor's area in points of a window whose scale is `window_scale`. Monitors that touch in pixels touch
    /// in these points, with no gap or overlap.
    pub fn points_at(&self, window_scale: f32) -> Rect {
        let scale = window_scale.max(f32::EPSILON);
        let px = self.rect_px();
        Rect::from_min_max(px.min / scale, px.max / scale)
    }
}
