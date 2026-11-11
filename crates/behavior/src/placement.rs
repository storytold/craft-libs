//! Where windows go on the desktop: where a saved window opens ([`place`]), which monitor a window is on, and where
//! the next window of a cascade opens.
//!
//! Units. Sizes and `NormalRect`s are logical points at interface zoom 1, so a window saved while the user ran the
//! interface at 150 % does not open smaller each launch. Positions are physical pixels in the desktop's coordinate
//! space, as [`Monitor`]s are. Cascades work in the points of one window.

use emath::{Rect, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};

use crate::monitor::{Monitor, MonitorKey};

/// How much of the window's top strip must lie on one monitor, in points on each axis, for the
/// user to still be able to grab it.
const BAR: f32 = 64.0;
const MAX_POINTS: f32 = 100_000.0;

/// A rectangle in logical points at zoom 1: top-left corner and size.
///
/// `pos` is `Placement::position_px` divided by `Placement::scale`; it is informational, and
/// `[0, 0]` when the position is unknown.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NormalRect {
    /// Top-left corner in points.
    pub pos: [f32; 2],
    /// Width and height in points.
    pub size: [f32; 2],
}

/// What to remember about a window, and what [`place`] returns for the app to open it with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Placement {
    /// The last window state that was neither maximized nor full screen.
    pub normal: NormalRect,
    /// Outer top-left corner in physical pixels; `None` where the platform does not tell (Wayland).
    #[serde(default)]
    pub position_px: Option<[i32; 2]>,
    /// Physical pixels per point of the monitor the window was on, without interface zoom.
    #[serde(default = "one")]
    pub scale: f32,
    /// The interface zoom factor while saving. Informational: `normal` is already at zoom 1.
    #[serde(default = "one")]
    pub zoom: f32,
    /// The window was maximized.
    #[serde(default)]
    pub maximized: bool,
    /// The window was full screen.
    #[serde(default)]
    pub fullscreen: bool,
    /// The monitor the window was on, if known.
    #[serde(default)]
    pub monitor: Option<MonitorKey>,
}

impl Placement {
    /// A window of `size` points, position unknown, on a monitor of `scale`, not maximized.
    pub fn new(size: [f32; 2], scale: f32) -> Self {
        make(size, None, scale, 1.0, None)
    }

    /// The record with out-of-range numbers repaired, or `None` when its size is no size.
    pub fn repaired(&self) -> Option<Placement> {
        if !self.normal.size.iter().all(|v| in_points(*v)) {
            return None;
        }
        let mut s = self.clone();
        if !s.normal.pos.iter().all(|v| v.is_finite()) {
            s.normal.pos = [0.0; 2];
        }
        if !(s.scale.is_finite() && s.scale > 0.0 && s.scale <= 16.0) {
            s.scale = 1.0;
        }
        if !(s.zoom.is_finite() && (0.1..=10.0).contains(&s.zoom)) {
            s.zoom = 1.0;
        }
        Some(s)
    }

    /// The outer top-left corner in desktop points at the saved scale, where the position is known.
    pub fn position(&self) -> Option<[f32; 2]> {
        let scale = self.scale.max(f32::EPSILON);
        self.position_px.map(|p| p.map(|v| v as f32 / scale))
    }
}

fn one() -> f32 {
    1.0
}

/// What the app wants when nothing (usable) is saved, and how the platform behaves.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Defaults {
    /// Size of a first window, in points.
    pub size: [f32; 2],
    /// The smallest window, in points. Yields to a monitor smaller than this.
    pub min: [f32; 2],
    /// Points a monitor loses to the taskbar, Dock or menu bar, per axis.
    pub screen_room: [f32; 2],
    /// Restore full screen. Off by default: most apps open as a window.
    pub restore_fullscreen: bool,
    /// The platform hides window positions (Wayland): none is applied or returned.
    pub no_position: bool,
}

impl Defaults {
    /// Defaults for a first window of `size` points and a minimum of `min` points.
    pub fn new(size: [f32; 2], min: [f32; 2]) -> Self {
        Self { size, min, screen_room: [0.0, 48.0], restore_fullscreen: false, no_position: false }
    }

    /// Set the room a monitor loses to the taskbar, Dock or menu bar.
    pub fn with_screen_room(mut self, room: [f32; 2]) -> Self {
        self.screen_room = room;
        self
    }

    /// Set whether full screen is restored.
    pub fn with_restore_fullscreen(mut self, on: bool) -> Self {
        self.restore_fullscreen = on;
        self
    }

    /// Set whether the platform hides window positions.
    pub fn with_no_position(mut self, on: bool) -> Self {
        self.no_position = on;
        self
    }
}

impl Monitor {
    fn safe_scale(&self) -> f32 {
        if self.scale.is_finite() && self.scale > 0.0 { self.scale } else { 1.0 }
    }

    fn points(&self) -> [f32; 2] {
        self.size_px.map(|s| s as f32 / self.safe_scale())
    }

    /// Position and size in pixels of the area windows may use.
    fn bounds(&self) -> ([i32; 2], [u32; 2]) {
        self.work_px.unwrap_or((self.pos_px, self.size_px))
    }

    /// Size in points of the area windows may use: the work area, else the monitor less `room`.
    fn usable(&self, room: [f32; 2]) -> [f32; 2] {
        let s = self.safe_scale();
        match self.work_px {
            Some((_, size)) => size.map(|v| (v as f32 / s).max(1.0)),
            None => {
                let p = self.points();
                std::array::from_fn(|i| (p[i] - room[i].max(0.0)).max(1.0))
            }
        }
    }

    /// `pos` moved the least that puts a window of `size` points fully in the usable area.
    fn keep_on(&self, pos: [i32; 2], size: [f32; 2]) -> [i32; 2] {
        let (bpos, bsize) = self.bounds();
        std::array::from_fn(|i| {
            let lo = i64::from(bpos[i]);
            let win = (f64::from(size[i]) * f64::from(self.safe_scale())).round() as i64;
            let hi = (lo + i64::from(bsize[i]) - win).max(lo);
            i32::try_from(i64::from(pos[i]).clamp(lo, hi)).unwrap_or(bpos[i])
        })
    }

    /// Area in points of the window's top strip (full width, up to 64 pt tall) lying on this
    /// monitor, if at least min(width, 64) × min(height, 64) pt of it does. A few pixels of slack
    /// absorb Windows' snapped offset of about -7 px.
    fn bar_overlap(&self, pos: [i32; 2], size: [f32; 2]) -> Option<f32> {
        const SLACK_PX: f64 = 8.0;
        let s = f64::from(self.safe_scale());
        let strip = [size[0], size[1].min(BAR)];
        let need = [size[0].min(BAR), size[1].min(BAR)];
        let mut got = [0.0f64; 2];
        for i in 0..2 {
            let a0 = f64::from(pos[i]);
            let a1 = a0 + f64::from(strip[i]) * s;
            let b0 = f64::from(self.pos_px[i]);
            let b1 = b0 + f64::from(self.size_px[i]);
            got[i] = (a1.min(b1) - a0.max(b0)).max(0.0);
        }
        (0..2).all(|i| got[i] + SLACK_PX >= f64::from(need[i]) * s).then(|| (got[0] / s * got[1] / s) as f32)
    }
}

/// Size no bigger than `room` and no smaller than `min` unless `room` is.
fn fit(size: [f32; 2], room: [f32; 2], min: [f32; 2]) -> [f32; 2] {
    std::array::from_fn(|i| size[i].min(room[i]).max(min[i].min(room[i])).max(1.0))
}

fn in_points(v: f32) -> bool {
    v.is_finite() && (1.0..=MAX_POINTS).contains(&v)
}

fn make(size: [f32; 2], pos_px: Option<[i32; 2]>, scale: f32, zoom: f32, monitor: Option<MonitorKey>) -> Placement {
    let pos = pos_px.map_or([0.0, 0.0], |p| p.map(|v| v as f32 / scale));
    Placement { normal: NormalRect { pos, size }, position_px: pos_px, scale, zoom, maximized: false, fullscreen: false, monitor }
}

/// Where a window opens: `saved` kept on a connected monitor, else the default size on `home`
/// (index into `monitors` of the monitor the system opens it on; the one at the origin, else the
/// first, when `None`).
///
/// - Nothing saved, or a saved size that is no size: [`Defaults::size`] fitted to the usable area of `home`;
///   maximized when the monitor is too small for it. The position is left to the system.
/// - The saved size is points at zoom 1 and is not scaled by `zoom`.
/// - `normal` is kept as saved while `maximized`; only the flag says it was maximized.
/// - Off screen: when less than 64×64 pt of the window's top strip lies on any one monitor, the
///   window moves to its saved monitor, else the one [`monitor_of`] finds for it, else the primary.
/// - The size is clamped to the usable area of the monitor it lands on and to `min`.
/// - A different `scale` than saved is taken from the monitor; the size stays in points.
/// - [`Defaults::no_position`]: no position in, none out.
/// - Full screen comes back only with [`Defaults::restore_fullscreen`].
/// - Without monitors (the platform lists none) nothing can be checked: the saved geometry is kept,
///   with the size raised to `min`.
pub fn place(saved: Option<&Placement>, monitors: &[Monitor], home: Option<usize>, d: &Defaults) -> Placement {
    let saved = saved.and_then(Placement::repaired);
    let primary = monitors.iter().find(|m| m.pos_px == [0, 0]).or(monitors.first());
    let home = home.and_then(|i| monitors.get(i)).or(primary);
    match (saved, home) {
        (None, None) => make(fit(d.size, [MAX_POINTS; 2], d.min), None, 1.0, 1.0, None),
        (None, Some(h)) => first_launch(h, d),
        (Some(s), None) => {
            let size = fit(s.normal.size, [MAX_POINTS; 2], d.min);
            let pos = if d.no_position { None } else { s.position_px };
            Placement { maximized: s.maximized, fullscreen: s.fullscreen && d.restore_fullscreen, ..make(size, pos, s.scale, s.zoom, s.monitor) }
        }
        (Some(s), Some(h)) => restore(&s, monitors, h, primary.unwrap_or(h), d),
    }
}

fn first_launch(home: &Monitor, d: &Defaults) -> Placement {
    let usable = home.usable(d.screen_room);
    let too_small = (0..2).any(|i| d.size[i] > usable[i]);
    let size = fit(d.size, usable, d.min);
    Placement { maximized: too_small, ..make(size, None, home.safe_scale(), 1.0, Some(home.key.clone())) }
}

fn restore(s: &Placement, monitors: &[Monitor], home: &Monitor, primary: &Monitor, d: &Defaults) -> Placement {
    let size = s.normal.size;
    let saved_pos = if d.no_position { None } else { s.position_px };
    let by_key = || s.monitor.as_ref().and_then(|k| monitors.iter().find(|m| &m.key == k));

    let mut visible = false;
    let target = match saved_pos {
        Some(p) => {
            // On mixed-DPI desktops pixel ranges overlap; the saved monitor decides first.
            let shown = monitors
                .iter()
                .filter_map(|m| m.bar_overlap(p, size).map(|a| (a, m)))
                .max_by(|a, b| {
                    let key = |m: &Monitor| s.monitor.as_ref() == Some(&m.key);
                    key(a.1).cmp(&key(b.1)).then(a.0.total_cmp(&b.0))
                })
                .map(|(_, m)| m);
            visible = shown.is_some();
            let window_px = Rect::from_min_size(pos2(p[0] as f32, p[1] as f32), vec2(size[0], size[1]) * s.scale);
            shown.or_else(by_key).unwrap_or_else(|| monitor_of(monitors, window_px).unwrap_or(primary))
        }
        None => by_key().unwrap_or(home),
    };

    let fitted = fit(size, target.usable(d.screen_room), d.min);
    let kept = visible && fitted == size;
    let pos = saved_pos.map(|p| if kept { p } else { target.keep_on(p, fitted) });
    // A kept position on the saved monitor keeps its saved scale: the pixel ranges of mixed-DPI
    // monitors overlap, and the position was measured at that scale.
    let scale = if kept && s.monitor.as_ref() == Some(&target.key) { s.scale } else { target.safe_scale() };
    Placement {
        maximized: s.maximized,
        fullscreen: s.fullscreen && d.restore_fullscreen,
        ..make(fitted, pos, scale, s.zoom, Some(target.key.clone()))
    }
}

/// The monitor the window at `window_px` (desktop pixels) is on: the one showing most of it or, when none shows it,
/// the nearest. By area, not by a corner or the centre: on Windows a maximized window overhangs its monitor on every
/// side. Monitors of no size are skipped. `None` only when there is no such monitor.
pub fn monitor_of(monitors: &[Monitor], window_px: Rect) -> Option<&Monitor> {
    let shown = |monitor: &Monitor| {
        let overlap = window_px.intersect(monitor.rect_px());
        overlap.width().max(0.0) * overlap.height().max(0.0)
    };
    let gap = |monitor: &Monitor| {
        let m = monitor.rect_px();
        let across = (m.min.x - window_px.max.x).max(window_px.min.x - m.max.x).max(0.0);
        let down = (m.min.y - window_px.max.y).max(window_px.min.y - m.max.y).max(0.0);
        across * across + down * down
    };
    monitors
        .iter()
        .filter(|monitor| monitor.size_px.iter().all(|v| *v > 0))
        .max_by(|a, b| shown(a).total_cmp(&shown(b)).then_with(|| gap(b).total_cmp(&gap(a))))
}

/// Where a window of `size` opens after the one at `previous`: one `step` right and down. An axis on which that
/// would leave `bounds` starts again at the edge of `bounds`, so a long run of windows walks down the screen and
/// wraps back to the top left. A window larger than `bounds` goes to the corner of `bounds`; its size never
/// changes.
pub fn cascade(previous: Rect, size: Vec2, bounds: Rect, step: f32) -> Rect {
    let axis = |previous: f32, length: f32, low: f32, high: f32| {
        let at = previous + step;
        if at < low || at + length > high { low } else { at }
    };
    let at = pos2(axis(previous.min.x, size.x, bounds.min.x, bounds.max.x), axis(previous.min.y, size.y, bounds.min.y, bounds.max.y));
    Rect::from_min_size(at, size)
}

/// `n` windows of `size` in a cascade inside `area`: the first at its top left, each next one [`cascade`]d from the
/// one before. The host passes its cascade step (`size.window.cascade`).
pub fn cascade_rects(area: Rect, size: Vec2, n: usize, step: f32) -> Vec<Rect> {
    let mut rects: Vec<Rect> = Vec::with_capacity(n);
    for _ in 0..n {
        let next = rects.last().map_or_else(|| Rect::from_min_size(area.min, size), |last| cascade(*last, size, area, step));
        rects.push(next);
    }
    rects
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(name: &str, pos: [i32; 2], size: [u32; 2], scale: f32) -> Monitor {
        Monitor::new(MonitorKey::new(name, size), pos, size, scale)
    }

    /// A 1920×1080 primary whose bottom 40 px belong to the taskbar.
    fn primary() -> Monitor {
        mon("A", [0, 0], [1920, 1080], 1.0).with_work_px([0, 0], [1920, 1040])
    }

    fn defaults() -> Defaults {
        Defaults::new([1200.0, 800.0], [400.0, 300.0])
    }

    fn saved(pos: [i32; 2], size: [f32; 2], scale: f32, m: &Monitor) -> Placement {
        make(size, Some(pos), scale, 1.0, Some(m.key.clone()))
    }

    /// Whether the window `p` lies wholly in the work area of `m`.
    fn in_work_area(p: &Placement, m: &Monitor) -> bool {
        let (pos, size) = m.bounds();
        let Some(at) = p.position_px else { return false };
        (0..2).all(|i| {
            let end = i64::from(at[i]) + (f64::from(p.normal.size[i]) * f64::from(p.scale)).round() as i64;
            at[i] >= pos[i] && end <= i64::from(pos[i]) + i64::from(size[i])
        })
    }

    #[test]
    fn a_window_on_a_monitor_that_is_gone_lands_in_the_primary_work_area() {
        let left = mon("L", [-1920, 0], [1920, 1080], 1.0);
        let right = mon("R", [1920, 0], [2560, 1440], 1.0);
        let s = saved([-1500, 700], [800.0, 600.0], 1.0, &left);
        let p = place(Some(&s), &[right, primary()], None, &defaults());
        assert_eq!(p.monitor, Some(primary().key));
        assert!(in_work_area(&p, &primary()), "{p:?}");
    }

    #[test]
    fn a_window_whose_bar_cannot_be_grabbed_comes_fully_into_the_work_area_and_a_grabbable_one_stays() {
        let m = primary();
        for at in [[1900, 100], [500, 1070], [-790, 300]] {
            let p = place(Some(&saved(at, [800.0, 600.0], 1.0, &m)), std::slice::from_ref(&m), None, &defaults());
            assert!(in_work_area(&p, &m), "from {at:?}: {p:?}");
        }
        for at in [[1820, 100], [500, 1000], [-700, 300]] {
            let p = place(Some(&saved(at, [800.0, 600.0], 1.0, &m)), std::slice::from_ref(&m), None, &defaults());
            assert_eq!(p.position_px, Some(at), "a grabbable bar stays");
        }
    }

    #[test]
    fn mixed_dpi_prefers_the_saved_monitor_and_keeps_its_scale() {
        let mac = mon("Mac", [0, 0], [3024, 1964], 2.0);
        let ext = mon("Ext", [1512, 0], [1920, 1080], 1.0);
        let s = saved([2000, 100], [800.0, 600.0], 2.0, &mac);
        let p = place(Some(&s), &[mac.clone(), ext], Some(0), &defaults());
        assert_eq!((p.monitor, p.scale, p.position_px), (Some(mac.key), 2.0, Some([2000, 100])));
    }

    #[test]
    fn a_window_is_on_the_monitor_showing_most_of_it() {
        let rect = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pos2(x, y), vec2(w, h));
        let left = mon("left", [-1920, 200], [1920, 1080], 1.0);
        let main = mon("main", [0, 0], [3840, 1600], 1.0);
        let below = mon("below", [718, 1600], [1280, 720], 1.0);
        let desktop = [left.clone(), main.clone(), below.clone()];
        assert_eq!(monitor_of(&desktop, rect(-8.0, -8.0, 3856.0, 1576.0)), Some(&main), "maximized, its corner outside");
        assert_eq!(monitor_of(&desktop, rect(-1928.0, 192.0, 1936.0, 1056.0)), Some(&left), "maximized, on the left");
        assert_eq!(monitor_of(&desktop, rect(800.0, 1700.0, 400.0, 300.0)), Some(&below), "offset and smaller");
        assert_eq!(monitor_of(&desktop, rect(-100.0, 300.0, 1000.0, 500.0)), Some(&main), "across two, mostly on one");
        assert_eq!(monitor_of(std::slice::from_ref(&main), rect(3400.0, 100.0, 800.0, 600.0)), Some(&main), "past the right edge");
    }
}
