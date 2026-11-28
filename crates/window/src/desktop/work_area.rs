//! Whether a window fits its monitor, and the part of a window the user can see and reach.
//!
//! egui doesn't expose the monitor's work area (the screen less the taskbar, Dock or panel), only
//! the monitor's size and the window's position on the desktop. [`TASKBAR`] is the room kept free
//! at the monitor's bottom edge. A window can be larger than its display, and menus sized by the
//! window's content rect then run under the taskbar: [`visible_part`] is the content rect cut to
//! the monitor less that room.

use egui::{Rect, Vec2, ViewportInfo, pos2, vec2};

/// Room kept free at the monitor's bottom edge for a taskbar or Dock egui can't see (the Windows
/// taskbar is 40–48 pt tall; a bottom Dock about as much).
// pc: crates/ui-egui/src/work_area.rs:17 `TASKBAR`.
pub const TASKBAR: f32 = 48.0;
/// Below this, the guess about where the monitor is went wrong: use the whole window.
const MIN_VISIBLE: f32 = 160.0;

/// Whether a window of outer size `outer` fits `monitor`: `outer.x <= monitor.x + 0.5` and
/// `outer.y <= monitor.y - TASKBAR + 0.5`.
// pc: crates/ui-egui/src/work_area.rs:100 `fits`.
pub fn fits(outer: Vec2, monitor: Vec2) -> bool {
    outer.x <= monitor.x + 0.5 && outer.y <= monitor.y - TASKBAR + 0.5
}

/// How the window is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowState {
    /// Neither maximized nor full screen.
    Normal,
    /// Maximized.
    Maximized,
    /// Full screen, which covers the taskbar on purpose: all of it can be seen.
    FullScreen,
}

impl WindowState {
    /// The state `info` reports.
    pub fn of(info: &ViewportInfo) -> Self {
        if info.fullscreen == Some(true) {
            Self::FullScreen
        } else if info.maximized == Some(true) {
            Self::Maximized
        } else {
            Self::Normal
        }
    }
}

/// The part of `content` that is visible on a monitor of size `monitor` for a window in `state` with inner rect
/// `inner` (the content's rect on the desktop), both in egui points; the whole of `content` when the monitor is
/// unknown or the input makes no sense.
// pc: crates/ui-egui/src/work_area.rs:24 `visible_part`.
pub fn visible_part(content: Rect, monitor: Option<Vec2>, inner: Option<Rect>, state: WindowState) -> Rect {
    let Some(m) = monitor else { return content };
    if state == WindowState::FullScreen || !m.x.is_finite() || !m.y.is_finite() || m.x < 1.0 || m.y < 1.0 {
        return content;
    }
    // A maximized window is usually inside the work area already, but not always (#3737): on X11
    // a window manager that doesn't reserve a panel's space (a panel without a strut) maximizes it
    // to the whole monitor, and the panel then covers its bottom. Where the window is known, it is
    // checked like any other; only a maximized window with no position (Wayland) is taken to fit.
    if state == WindowState::Maximized && inner.is_none() {
        return content;
    }
    // #3737: Wayland never tells a window where it is, so egui has no `inner_rect` there, only the
    // monitor's size. A window bigger than the monitor less the taskbar still runs under it
    // wherever it is: at best its content starts at the monitor's top, so at most that much of it
    // can be seen.
    let Some(inner) = inner else {
        let room = vec2(m.x, m.y - TASKBAR);
        return trusted(content, Rect::from_min_size(content.min, room));
    };
    if !inner.min.is_finite() || !inner.max.is_finite() {
        return content;
    }
    // Only the monitor's size is known, not where it is on the desktop: assume monitors tile the
    // desktop by their size (true for one monitor, and for side-by-side ones of the same size).
    let c = inner.center();
    let origin = pos2((c.x / m.x).floor() * m.x, (c.y / m.y).floor() * m.y);
    let screen = Rect::from_min_size(origin, m);
    let usable = Rect::from_min_max(screen.min, pos2(screen.max.x, screen.max.y - TASKBAR));
    // Desktop → window coordinates.
    let shift = content.min - inner.min;
    trusted(content, usable.translate(shift))
}

/// `content` cut to `usable`, unless that leaves too little to be a sensible guess.
fn trusted(content: Rect, usable: Rect) -> Rect {
    let visible = content.intersect(usable);
    if !visible.is_positive() || visible.height() < MIN_VISIBLE.min(content.height()) || visible.width() < MIN_VISIBLE.min(content.width()) {
        return content;
    }
    visible
}

#[cfg(test)]
mod tests {
    use super::*;
    use WindowState::{FullScreen, Maximized, Normal};

    const MONITOR: Vec2 = vec2(1366.0, 768.0);

    fn window(x: f32, y: f32, w: f32, h: f32) -> (Rect, Rect) {
        (Rect::from_min_size(pos2(0.0, 0.0), vec2(w, h)), Rect::from_min_size(pos2(x, y), vec2(w, h)))
    }

    #[test]
    fn a_window_taller_than_its_display_is_cut_at_the_taskbar() {
        // 1440 × 900 at the desktop origin on 1366 × 768: right and bottom are off-screen.
        let (content, inner) = window(0.0, 0.0, 1440.0, 900.0);
        assert_eq!(visible_part(content, Some(MONITOR), Some(inner), Normal), Rect::from_min_max(pos2(0.0, 0.0), pos2(1366.0, 768.0 - TASKBAR)));
        // Below a 31 pt title bar, a full-height window reaches under the taskbar.
        let (content, inner) = window(0.0, 31.0, 1366.0, 737.0);
        assert_eq!(visible_part(content, Some(MONITOR), Some(inner), Normal).bottom(), 768.0 - TASKBAR - 31.0);
        // A small window clear of the taskbar is all visible.
        let (content, inner) = window(100.0, 80.0, 800.0, 500.0);
        assert_eq!(visible_part(content, Some(MONITOR), Some(inner), Normal), content);
        // On a second monitor to the right, the same.
        let (content, inner) = window(1366.0 + 100.0, 0.0, 1440.0, 900.0);
        assert_eq!(visible_part(content, Some(MONITOR), Some(inner), Normal).bottom(), 768.0 - TASKBAR);
    }

    #[test]
    fn maximized_unknown_or_nonsense_input_keeps_the_window() {
        let (content, inner) = window(0.0, 0.0, 1440.0, 900.0);
        assert_eq!(visible_part(content, Some(MONITOR), Some(inner), FullScreen), content, "full screen covers the taskbar on purpose");
        assert_eq!(visible_part(content, None, Some(inner), Normal), content);
        assert_eq!(visible_part(content, Some(vec2(f32::NAN, 768.0)), Some(inner), Normal), content);
        assert_eq!(visible_part(content, Some(vec2(0.0, 0.0)), Some(inner), Normal), content);
        assert_eq!(visible_part(content, Some(MONITOR), Some(Rect::from_min_size(pos2(f32::INFINITY, 0.0), vec2(10.0, 10.0))), Normal), content);
        // A guess that would leave almost nothing visible is not trusted.
        let (content, inner) = window(0.0, 600.0, 1000.0, 300.0);
        assert_eq!(visible_part(content, Some(MONITOR), Some(inner), Normal), content);
    }

    #[test]
    fn a_maximized_window_under_a_panel_is_still_cut_at_the_taskbar() {
        // #3737, X11: the window manager maximized the window to the whole 1920 × 1080 monitor
        // without reserving the bottom panel's space, so the panel covers it.
        for scale in [1.0_f32, 1.5] {
            let monitor = vec2(1920.0, 1080.0) / scale;
            let content = Rect::from_min_size(pos2(0.0, 0.0), monitor);
            let bottom = monitor.y - TASKBAR;
            assert_eq!(visible_part(content, Some(monitor), Some(content), Maximized).bottom(), bottom, "at {scale}×");
            // A window manager that reserves the panel: the maximized window already ends above it.
            let fitted = Rect::from_min_size(pos2(0.0, 0.0), vec2(monitor.x, bottom));
            assert_eq!(visible_part(fitted, Some(monitor), Some(fitted), Maximized), fitted, "at {scale}×");
            // Full screen is meant to cover the panel.
            assert_eq!(visible_part(content, Some(monitor), Some(content), FullScreen), content, "at {scale}×");
        }
    }

    #[test]
    fn an_unknown_window_position_still_keeps_clear_of_the_taskbar() {
        // #3737, Wayland: only the monitor's size is known. A window taller than the monitor less
        // the taskbar can be seen at most down to there, wherever it is.
        let content = Rect::from_min_size(pos2(0.0, 0.0), vec2(1366.0, 740.0));
        assert_eq!(visible_part(content, Some(MONITOR), None, Normal), Rect::from_min_max(pos2(0.0, 0.0), pos2(1366.0, 768.0 - TASKBAR)));
        let big = Rect::from_min_size(pos2(0.0, 0.0), vec2(1440.0, 900.0));
        assert_eq!(visible_part(big, Some(MONITOR), None, Normal), Rect::from_min_max(pos2(0.0, 0.0), pos2(1366.0, 768.0 - TASKBAR)));
        // One that fits, or a maximized one, is all visible.
        let small = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 600.0));
        assert_eq!(visible_part(small, Some(MONITOR), None, Normal), small);
        assert_eq!(visible_part(content, Some(MONITOR), None, Maximized), content);
        assert_eq!(visible_part(content, Some(MONITOR), None, FullScreen), content);
    }
}
