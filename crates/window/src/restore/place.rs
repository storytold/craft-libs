//! The model and the pure placement rules: saved geometry plus the connected monitors in, the
//! geometry to open with out.
//!
//! Units. Sizes and `NormalRect`s are logical points at interface zoom 1, so a window saved while the
//! user ran the interface at 150 % does not open smaller each launch. Positions are physical
//! pixels in the desktop's coordinate space (negative for a monitor left of or above the primary).

use serde::{Deserialize, Serialize};

/// How much of the window's top strip must lie on one monitor, in points on each axis, for the
/// user to still be able to grab it.
const BAR: f32 = 64.0;
const MAX_POINTS: f32 = 100_000.0;

/// A rectangle in logical points at zoom 1: top-left corner and size. (Not `craft_window::Rect`,
/// which is the cascade module's.)
///
/// `pos` is `Placement::position_px` divided by `Placement::scale`; it is informational, and
/// `[0, 0]` when the position is unknown.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct NormalRect {
    pub pos: [f32; 2],
    pub size: [f32; 2],
}

/// Identifies a monitor across launches: the name the platform reports and its size in pixels.
///
/// The size is part of the key because names repeat (`"Built-in Display"`) or change (`DP-1`
/// after a re-plug), and a monitor switched to another resolution is a different place to put a
/// window. A key that no longer matches falls back to the nearest connected monitor.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MonitorKey {
    pub name: String,
    pub size_px: [u32; 2],
}

impl MonitorKey {
    pub fn new(name: impl Into<String>, size_px: [u32; 2]) -> Self {
        Self { name: name.into(), size_px }
    }
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
    #[serde(default)]
    pub maximized: bool,
    #[serde(default)]
    pub fullscreen: bool,
    #[serde(default)]
    pub monitor: Option<MonitorKey>,
}

impl Placement {
    /// A window of `size` points, position unknown, on a monitor of `scale`, not maximized.
    pub fn new(size: [f32; 2], scale: f32) -> Self {
        make(size, None, scale, 1.0, None)
    }
}

fn one() -> f32 {
    1.0
}

/// A connected monitor.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Monitor {
    pub key: MonitorKey,
    /// Top-left corner in the desktop's pixel space.
    pub pos_px: [i32; 2],
    pub size_px: [u32; 2],
    /// Physical pixels per point.
    pub scale: f32,
    /// The part not taken by the taskbar, Dock or menu bar: top-left corner and size in pixels.
    /// When set, it replaces [`Defaults::screen_room`] for this monitor.
    pub work_px: Option<([i32; 2], [u32; 2])>,
}

impl Monitor {
    pub fn new(key: MonitorKey, pos_px: [i32; 2], size_px: [u32; 2], scale: f32) -> Self {
        Self { key, pos_px, size_px, scale, work_px: None }
    }

    /// Sets the usable area (see [`work_px`](Self::work_px)).
    pub fn with_work_px(mut self, pos_px: [i32; 2], size_px: [u32; 2]) -> Self {
        self.work_px = Some((pos_px, size_px));
        self
    }
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
    pub fn new(size: [f32; 2], min: [f32; 2]) -> Self {
        Self { size, min, screen_room: [0.0, 48.0], restore_fullscreen: false, no_position: false }
    }

    pub fn with_screen_room(mut self, room: [f32; 2]) -> Self {
        self.screen_room = room;
        self
    }

    pub fn with_restore_fullscreen(mut self, on: bool) -> Self {
        self.restore_fullscreen = on;
        self
    }

    pub fn with_no_position(mut self, on: bool) -> Self {
        self.no_position = on;
        self
    }
}

impl Monitor {
    fn scale(&self) -> f32 {
        if self.scale.is_finite() && self.scale > 0.0 { self.scale } else { 1.0 }
    }

    fn points(&self) -> [f32; 2] {
        self.size_px.map(|s| s as f32 / self.scale())
    }

    /// Position and size in pixels of the area windows may use.
    fn bounds(&self) -> ([i32; 2], [u32; 2]) {
        self.work_px.unwrap_or((self.pos_px, self.size_px))
    }

    /// Size in points of the area windows may use: the work area, else the monitor less `room`.
    fn usable(&self, room: [f32; 2]) -> [f32; 2] {
        let s = self.scale();
        match self.work_px {
            Some((_, size)) => size.map(|v| (v as f32 / s).max(1.0)),
            None => {
                let p = self.points();
                std::array::from_fn(|i| (p[i] - room[i].max(0.0)).max(1.0))
            }
        }
    }

    fn centre_px(&self) -> [f64; 2] {
        std::array::from_fn(|i| f64::from(self.pos_px[i]) + f64::from(self.size_px[i]) / 2.0)
    }

    /// `pos` moved the least that puts a window of `size` points fully in the usable area.
    fn keep_on(&self, pos: [i32; 2], size: [f32; 2]) -> [i32; 2] {
        let (bpos, bsize) = self.bounds();
        std::array::from_fn(|i| {
            let lo = i64::from(bpos[i]);
            let win = (f64::from(size[i]) * f64::from(self.scale())).round() as i64;
            let hi = (lo + i64::from(bsize[i]) - win).max(lo);
            i32::try_from(i64::from(pos[i]).clamp(lo, hi)).unwrap_or(bpos[i])
        })
    }

    /// Area in points of the window's top strip (full width, up to 64 pt tall) lying on this
    /// monitor, if at least min(width, 64) × min(height, 64) pt of it does. A few pixels of slack
    /// absorb Windows' snapped offset of about -7 px.
    fn bar_overlap(&self, pos: [i32; 2], size: [f32; 2]) -> Option<f32> {
        const SLACK_PX: f64 = 8.0;
        let s = f64::from(self.scale());
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

/// The saved record with out-of-range numbers repaired, or `None` when its size is no size.
pub(super) fn sane(s: &Placement) -> Option<Placement> {
    if !s.normal.size.iter().all(|v| in_points(*v)) {
        return None;
    }
    let mut s = s.clone();
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

fn make(size: [f32; 2], pos_px: Option<[i32; 2]>, scale: f32, zoom: f32, monitor: Option<MonitorKey>) -> Placement {
    let pos = pos_px.map_or([0.0, 0.0], |p| p.map(|v| v as f32 / scale));
    Placement { normal: NormalRect { pos, size }, position_px: pos_px, scale, zoom, maximized: false, fullscreen: false, monitor }
}

/// Where a window opens: `saved` kept on a connected monitor, else the default size on `home`
/// (index into `monitors` of the monitor the system opens it on; the one at the origin, else the
/// first, when `None`).
///
/// - Nothing saved: [`Defaults::size`] fitted to the usable area of `home`; maximized when the
///   monitor is too small for it. The position is left to the system.
/// - The saved size is points at zoom 1 and is not scaled by `zoom`.
/// - `normal` is kept as saved while `maximized`; only the flag says it was maximized.
/// - Off screen: when less than 64×64 pt of the window's top strip lies on any one monitor, the
///   window moves to its saved monitor, else the one nearest its centre, else the primary.
/// - The size is clamped to the usable area of the monitor it lands on and to `min`.
/// - A different `scale` than saved is taken from the monitor; the size stays in points.
/// - [`Defaults::no_position`]: no position in, none out.
/// - Full screen comes back only with [`Defaults::restore_fullscreen`].
/// - Without monitors (the platform lists none) nothing can be checked: the saved geometry is kept,
///   with the size raised to `min`.
pub fn place(saved: Option<&Placement>, monitors: &[Monitor], home: Option<usize>, d: &Defaults) -> Placement {
    let saved = saved.and_then(sane);
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
    Placement { maximized: too_small, ..make(size, None, home.scale(), 1.0, Some(home.key.clone())) }
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
            shown.or_else(by_key).unwrap_or_else(|| nearest(monitors, p, size, s.scale).unwrap_or(primary))
        }
        None => by_key().unwrap_or(home),
    };

    let fitted = fit(size, target.usable(d.screen_room), d.min);
    let kept = visible && fitted == size;
    let pos = saved_pos.map(|p| if kept { p } else { target.keep_on(p, fitted) });
    // A kept position on the saved monitor keeps its saved scale: the pixel ranges of mixed-DPI
    // monitors overlap, and the position was measured at that scale.
    let scale = if kept && s.monitor.as_ref() == Some(&target.key) { s.scale } else { target.scale() };
    Placement {
        maximized: s.maximized,
        fullscreen: s.fullscreen && d.restore_fullscreen,
        ..make(fitted, pos, scale, s.zoom, Some(target.key.clone()))
    }
}

/// The monitor whose centre is closest to the window's centre.
fn nearest(monitors: &[Monitor], pos: [i32; 2], size: [f32; 2], scale: f32) -> Option<&Monitor> {
    let c: [f64; 2] = std::array::from_fn(|i| f64::from(pos[i]) + f64::from(size[i]) * f64::from(scale) / 2.0);
    let dist = |m: &Monitor| {
        let mc = m.centre_px();
        (mc[0] - c[0]).powi(2) + (mc[1] - c[1]).powi(2)
    };
    monitors.iter().min_by(|a, b| dist(a).total_cmp(&dist(b)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn mon(name: &str, pos: [i32; 2], size: [u32; 2], scale: f32) -> Monitor {
        Monitor::new(MonitorKey::new(name, size), pos, size, scale)
    }

    fn hd() -> Monitor {
        mon("A", [0, 0], [1920, 1080], 1.0)
    }

    fn defaults() -> Defaults {
        let mut d = Defaults::new([1200.0, 800.0], [400.0, 300.0]);
        d.screen_room = [0.0, 40.0];
        d
    }

    fn saved(pos: Option<[i32; 2]>, size: [f32; 2], scale: f32, m: &Monitor) -> Placement {
        let mut p = make(size, pos, scale, 1.0, Some(m.key.clone()));
        p.maximized = false;
        p
    }

    #[test]
    fn first_launch_uses_default_size_and_leaves_position_to_the_system() {
        let p = place(None, &[hd()], Some(0), &defaults());
        assert_eq!(p.normal.size, [1200.0, 800.0]);
        assert_eq!(p.position_px, None);
        assert!(!p.maximized);
    }

    #[test]
    fn first_launch_on_a_small_monitor_fits_and_maximizes() {
        let small = mon("S", [0, 0], [1280, 720], 1.0);
        let p = place(None, &[small], None, &defaults());
        assert_eq!(p.normal.size, [1200.0, 680.0]);
        assert!(p.maximized);
    }

    #[test]
    fn monitor_gone_moves_the_window_to_a_connected_one() {
        let gone = mon("Gone", [1920, 0], [1920, 1080], 1.0);
        let s = saved(Some([2500, 100]), [800.0, 600.0], 1.0, &gone);
        let p = place(Some(&s), &[hd()], Some(0), &defaults());
        assert_eq!(p.monitor, Some(hd().key));
        let [x, y] = p.position_px.unwrap();
        assert!((0..=1920 - 800).contains(&x) && (0..=1080 - 600).contains(&y));
    }

    #[test]
    fn window_dragged_off_screen_comes_back_but_a_visible_bar_stays() {
        let m = hd();
        let off = saved(Some([1900, 100]), [800.0, 600.0], 1.0, &m);
        let p = place(Some(&off), std::slice::from_ref(&m), Some(0), &defaults());
        assert_eq!(p.position_px, Some([1120, 100]));
        // 100 pt of the bar still on the monitor: untouched.
        let ok = saved(Some([1820, 100]), [800.0, 600.0], 1.0, &m);
        assert_eq!(place(Some(&ok), &[m], Some(0), &defaults()).position_px, Some([1820, 100]));
    }

    #[test]
    fn mixed_dpi_prefers_the_saved_monitor_and_keeps_its_scale() {
        let mac = mon("Mac", [0, 0], [3024, 1964], 2.0);
        let ext = mon("Ext", [1512, 0], [1920, 1080], 1.0);
        let s = saved(Some([2000, 100]), [800.0, 600.0], 2.0, &mac);
        let p = place(Some(&s), &[mac.clone(), ext], Some(0), &defaults());
        assert_eq!((p.monitor, p.scale, p.position_px), (Some(mac.key), 2.0, Some([2000, 100])));
    }

    #[test]
    fn bar_is_a_strip_not_a_corner() {
        let m = hd();
        // Left edge: 100 px of an 800 pt window show on the right of x = -700; the strip is wide enough.
        let s = saved(Some([-700, 100]), [800.0, 600.0], 1.0, &m);
        assert_eq!(place(Some(&s), std::slice::from_ref(&m), Some(0), &defaults()).position_px, Some([-700, 100]));
        // Only 30 px show: moved back.
        let s = saved(Some([-770, 100]), [800.0, 600.0], 1.0, &m);
        assert_eq!(place(Some(&s), std::slice::from_ref(&m), Some(0), &defaults()).position_px, Some([0, 100]));
        // Windows' snapped -7 px offset on the left or top is fine.
        let s = saved(Some([-7, -7]), [800.0, 600.0], 1.0, &m);
        assert_eq!(place(Some(&s), std::slice::from_ref(&m), Some(0), &defaults()).position_px, Some([-7, -7]));
        // Top edge: the strip is 64 pt tall; 40 pt of it visible is not enough.
        let s = saved(Some([100, -24]), [800.0, 600.0], 1.0, &m);
        assert_eq!(place(Some(&s), std::slice::from_ref(&m), Some(0), &defaults()).position_px, Some([100, 0]));
    }

    #[test]
    fn work_area_replaces_screen_room() {
        let m = hd().with_work_px([0, 0], [1920, 1000]);
        let p = place(None, &[m], Some(0), &Defaults::new([1200.0, 1100.0], [400.0, 300.0]));
        assert_eq!(p.normal.size, [1200.0, 1000.0]);
        assert!(p.maximized);
    }

    #[test]
    fn bar_above_the_top_edge_counts_as_off_screen() {
        let m = hd();
        let s = saved(Some([100, -50]), [800.0, 600.0], 1.0, &m);
        assert_eq!(place(Some(&s), &[m], Some(0), &defaults()).position_px, Some([100, 0]));
    }

    #[test]
    fn scale_change_keeps_points_and_takes_the_new_scale() {
        let old = hd();
        let hidpi = mon("B", [0, 0], [3840, 2160], 2.0);
        let s = saved(Some([200, 100]), [1000.0, 700.0], 1.0, &old);
        let p = place(Some(&s), std::slice::from_ref(&hidpi), Some(0), &defaults());
        assert_eq!(p.normal.size, [1000.0, 700.0]);
        assert_eq!(p.scale, 2.0);
        assert_eq!(p.monitor, Some(hidpi.key));
        assert_eq!(p.position_px, Some([200, 100]));
        assert_eq!(p.normal.pos, [100.0, 50.0]);
    }

    #[test]
    fn smaller_monitor_clamps_to_usable_area_and_min() {
        let big = mon("Big", [0, 0], [3840, 2160], 1.0);
        let s = saved(Some([100, 100]), [3000.0, 1800.0], 1.0, &big);
        let small = mon("S", [0, 0], [1280, 720], 1.0);
        let p = place(Some(&s), std::slice::from_ref(&small), Some(0), &defaults());
        assert_eq!(p.normal.size, [1280.0, 680.0]);
        assert_eq!(p.position_px, Some([0, 40]));
        // A monitor below `min`: the monitor wins.
        let tiny = mon("T", [0, 0], [320, 240], 1.0);
        let p = place(Some(&s), &[tiny], Some(0), &defaults());
        assert_eq!(p.normal.size, [320.0, 200.0]);
        // `min` raises a sliver saved size.
        let sliver = saved(None, [50.0, 50.0], 1.0, &small);
        assert_eq!(place(Some(&sliver), &[small], Some(0), &defaults()).normal.size, [400.0, 300.0]);
    }

    #[test]
    fn negative_origin_monitors_are_valid_places() {
        let left = mon("L", [-1920, 0], [1920, 1080], 1.0);
        let above = mon("U", [0, -1080], [1920, 1080], 1.0);
        let ms = [hd(), left.clone(), above.clone()];
        let s = saved(Some([-1500, 100]), [800.0, 600.0], 1.0, &left);
        assert_eq!(place(Some(&s), &ms, Some(0), &defaults()).position_px, Some([-1500, 100]));
        let s = saved(Some([300, -900]), [800.0, 600.0], 1.0, &above);
        assert_eq!(place(Some(&s), &ms, Some(0), &defaults()).position_px, Some([300, -900]));
        // The left monitor is unplugged: back onto the primary, not stuck at a negative x.
        let s = saved(Some([-1500, 100]), [800.0, 600.0], 1.0, &left);
        assert_eq!(place(Some(&s), &[hd()], Some(0), &defaults()).position_px, Some([0, 100]));
    }

    #[test]
    fn nearest_monitor_wins_when_the_saved_one_is_gone() {
        let right = mon("R", [1920, 0], [1920, 1080], 1.0);
        let far = mon("F", [5000, 0], [1920, 1080], 1.0);
        let gone = mon("Gone", [3840, 0], [1920, 1080], 1.0);
        let s = saved(Some([6000, 100]), [800.0, 600.0], 1.0, &gone);
        let p = place(Some(&s), &[hd(), right, far.clone()], Some(0), &defaults());
        assert_eq!(p.monitor, Some(far.key));
    }

    #[test]
    fn wayland_neither_reads_nor_returns_a_position() {
        let mut d = defaults();
        d.no_position = true;
        let m = hd();
        let s = saved(Some([5000, 5000]), [800.0, 600.0], 1.0, &m);
        let p = place(Some(&s), std::slice::from_ref(&m), Some(0), &d);
        assert_eq!(p.position_px, None);
        assert_eq!(p.normal.size, [800.0, 600.0]);
        assert_eq!(place(None, &[m], Some(0), &d).position_px, None);
    }

    #[test]
    fn size_saved_at_zoom_one_does_not_shrink_with_zoom() {
        let m = hd();
        let mut s = saved(Some([100, 100]), [1000.0, 700.0], 1.0, &m);
        s.zoom = 1.5;
        let p = place(Some(&s), &[m], Some(0), &defaults());
        assert_eq!(p.normal.size, [1000.0, 700.0]);
        assert_eq!(p.zoom, 1.5);
    }

    #[test]
    fn maximized_keeps_the_normal_rect() {
        let m = hd();
        let mut s = saved(Some([100, 100]), [900.0, 600.0], 1.0, &m);
        s.maximized = true;
        let p = place(Some(&s), &[m], Some(0), &defaults());
        assert!(p.maximized);
        assert_eq!((p.normal.size, p.position_px), ([900.0, 600.0], Some([100, 100])));
    }

    #[test]
    fn full_screen_needs_the_opt_in() {
        let m = hd();
        let mut s = saved(Some([100, 100]), [900.0, 600.0], 1.0, &m);
        s.fullscreen = true;
        let mut d = defaults();
        assert!(!place(Some(&s), std::slice::from_ref(&m), Some(0), &d).fullscreen);
        d.restore_fullscreen = true;
        assert!(place(Some(&s), &[m], Some(0), &d).fullscreen);
    }

    #[test]
    fn nonsense_saved_numbers_fall_back_to_a_first_launch() {
        let m = hd();
        let mut s = saved(Some([0, 0]), [f32::NAN, 600.0], 1.0, &m);
        assert_eq!(place(Some(&s), std::slice::from_ref(&m), Some(0), &defaults()).normal.size, [1200.0, 800.0]);
        s.normal.size = [800.0, 600.0];
        s.scale = 0.0;
        s.zoom = 99.0;
        let p = place(Some(&s), &[m], Some(0), &defaults());
        assert_eq!((p.scale, p.zoom), (1.0, 1.0));
    }

    #[test]
    fn no_monitors_keeps_the_saved_geometry() {
        let m = hd();
        let s = saved(Some([10, 20]), [800.0, 600.0], 1.0, &m);
        let p = place(Some(&s), &[], None, &defaults());
        assert_eq!((p.normal.size, p.position_px), ([800.0, 600.0], Some([10, 20])));
    }
}
