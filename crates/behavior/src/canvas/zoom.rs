//! Zoom rules as pure functions on a [`CanvasView`]: the zoom scheme's range and Zoom In / Zoom Out rungs, Fit on
//! Screen and Fill Screen, a zoom about a point and the anchors of wheel zoom.
//!
//! Zoom levels here are **device zoom**: device pixels per document unit, so 1 is 100 % at any interface scale.
//! [`CanvasView::zoom`] is screen points per document unit; the two convert with [`device`] and
//! [`CanvasView::points_per_unit`]. Functions that change a view take it `&mut`: the app copies `center` and
//! `zoom` back into its own view type.
//!
//! Every way of setting the zoom (wheel, Zoom tool, menu, zoom field, navigator, automation) clamps through
//! [`ZoomScheme::clamp`], so no control can pull the zoom into a narrower range of its own.

use super::view::CanvasView;
use emath::{Pos2, Rect as ScreenRect, Vec2 as ScreenVec};
use kurbo::{Point, Rect, Size, Vec2};
use std::ops::RangeInclusive;

/// A zoom within 0.1 % of a rung counts as on it.
const RUNG_TOLERANCE_UP: f64 = 1.001;
const RUNG_TOLERANCE_DOWN: f64 = 0.999;

/// Photoshop's Zoom In / Zoom Out rungs (1 = 100 %), measured on Photoshop 25.4.
const PHOTOSHOP_RUNGS: [f64; 33] = [
    0.001,
    0.002,
    0.003,
    0.004,
    0.005,
    0.007,
    0.01,
    0.015,
    0.02,
    0.03,
    0.04,
    0.05,
    0.0625,
    1.0 / 12.0,
    0.125,
    1.0 / 6.0,
    0.25,
    1.0 / 3.0,
    0.5,
    2.0 / 3.0,
    1.0,
    2.0,
    3.0,
    4.0,
    5.0,
    6.0,
    7.0,
    8.0,
    12.0,
    16.0,
    32.0,
    64.0,
    128.0,
];

/// The least zoom: the document's shorter side that many device pixels, or a fixed zoom in scheme units.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ZoomFloor {
    /// The document's shorter side is this many device pixels.
    ShorterSide(f64),
    /// A fixed zoom in scheme units.
    Fixed(f64),
}

/// A zoom range and its Zoom In and Out rungs in scheme units (1 = its 100 %); `hundred` is the device zoom of
/// 100 %. A step goes from any zoom to the next rung (880 %: in to 1200 %, out to 800 %).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ZoomScheme {
    /// The Zoom In and Out rungs, lowest first.
    pub rungs: &'static [f64],
    /// The greatest zoom.
    pub max: f64,
    /// The least zoom.
    pub floor: ZoomFloor,
    /// The device zoom of 100 %.
    pub hundred: f64,
}

impl ZoomScheme {
    /// Photoshop's: 33 rungs, up to 12800 %, the shorter side 2 px at the least.
    pub const PHOTOSHOP: ZoomScheme = ZoomScheme { rungs: &PHOTOSHOP_RUNGS, max: 128.0, floor: ZoomFloor::ShorterSide(2.0), hundred: 1.0 };

    /// Illustrator's: 26 stops from 3.13 % to 64000 %.
    pub const ILLUSTRATOR: ZoomScheme = ZoomScheme {
        rungs: &[
            0.0313, 0.0417, 0.0625, 0.0833, 0.125, 0.1667, 0.25, 0.3333, 0.5, 0.6667, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0, 32.0,
            48.0, 64.0, 128.0, 256.0, 640.0,
        ],
        max: 640.0,
        floor: ZoomFloor::Fixed(0.0313),
        hundred: 1.0,
    };

    /// The scheme with `device` as the device zoom of 100 %.
    pub const fn with_hundred(self, device: f64) -> Self {
        ZoomScheme { hundred: device, ..self }
    }

    /// The least device zoom for a document of `size`. An empty size (a view before its first frame) gives the lowest
    /// rung.
    pub fn min(&self, size: Size) -> f64 {
        match self.floor {
            ZoomFloor::ShorterSide(pixels) => match size.width.min(size.height) {
                short if short.is_finite() && short > 0.0 => (pixels / short).min(self.max_device()),
                _ => self.rungs.first().map_or(self.hundred, |rung| rung * self.hundred),
            },
            ZoomFloor::Fixed(zoom) => zoom * self.hundred,
        }
    }

    /// The greatest device zoom.
    pub fn max_device(&self) -> f64 {
        self.max * self.hundred
    }

    /// `device` within the range for a document of `size`; a zoom that is not a finite number becomes 100 %.
    pub fn clamp(&self, device: f64, size: Size) -> f64 {
        if device.is_finite() { device.min(self.max_device()).max(self.min(size)) } else { self.hundred }
    }

    /// Zoom In (`dir` > 0) or Zoom Out (`dir` < 0) from `device`: the next rung past it (a zoom within 0.1 % of a rung
    /// counts as on it), within the document's range.
    pub fn step(&self, device: f64, dir: i32, size: Size) -> f64 {
        self.step_in_range(device, dir, self.min(size)..=self.max_device())
    }

    /// [`step`](Self::step) within an explicit range of device zoom (a navigator or a zoom field with bounds of its
    /// own).
    pub fn step_in_range(&self, device: f64, dir: i32, range: RangeInclusive<f64>) -> f64 {
        let (lo, hi) = range.into_inner();
        let z = if device.is_finite() { device.min(hi).max(lo) } else { self.hundred };
        let rungs = self.rungs.iter().map(|rung| rung * self.hundred);
        let next = if dir > 0 {
            rungs.clone().find(|s| *s > z * RUNG_TOLERANCE_UP).unwrap_or(hi)
        } else {
            rungs.rev().find(|s| *s < z * RUNG_TOLERANCE_DOWN).unwrap_or(z)
        };
        if next.is_finite() { next.min(hi).max(lo) } else { z }
    }

    /// A zoom field's range in percent: the document's range widened to include `device` itself. A field narrower than
    /// the zoom would clamp it and write it back every pass.
    pub fn percent_range(&self, device: f64, size: Size) -> RangeInclusive<f64> {
        let z = if device.is_finite() { device } else { self.hundred };
        self.percent(self.min(size).min(z))..=self.percent(self.max_device().max(z))
    }

    /// `device` zoom as percent of the scheme's 100 %.
    pub fn percent(&self, device: f64) -> f64 {
        device / self.hundred * 100.0
    }

    /// The device zoom of `percent` of the scheme's 100 %.
    pub fn device(&self, percent: f64) -> f64 {
        percent / 100.0 * self.hundred
    }
}

/// `view.zoom` as device zoom (1 = 100 %) at `pixels_per_point`.
pub fn device(view: &CanvasView, pixels_per_point: f32) -> f64 {
    view.zoom * f64::from(pixels_per_point)
}

/// `view` at `device` zoom (1 = 100 %), clamped to the zoom range of a document of `size`, with its centre kept.
pub fn with_device(view: &CanvasView, device: f64, size: Size, pixels_per_point: f32, scheme: &ZoomScheme) -> CanvasView {
    let mut next = *view;
    next.zoom = CanvasView::points_per_unit(scheme.clamp(device, size), pixels_per_point);
    next
}

/// The document's width (with the pixel aspect) and height as the view draws them, and the
/// absolute sine and cosine of its rotation.
fn shown_extent(view: &CanvasView, size: Size) -> (f64, f64, f64, f64) {
    let aspect = if view.aspect.is_finite() && view.aspect > 0.0 { view.aspect } else { 1.0 };
    let (sin, cos) = view.rotation.to_radians().sin_cos();
    let exact = |v: f64| if v.abs() < 1e-12 { 0.0 } else { v.abs() };
    (size.width * aspect, size.height, exact(sin), exact(cos))
}

/// Fit on Screen: centre the document and zoom so all of it shows inside `view.rect` less `margin` points on
/// each side, never past 100 % (one document pixel per device pixel, so a 150 % display does not open
/// documents magnified). `size` is in document units; the view's [`aspect`](CanvasView::aspect) and rotation
/// are taken into account.
pub fn fit(view: &mut CanvasView, size: Size, pixels_per_point: f32, margin: f64, scheme: &ZoomScheme) {
    fit_up_to(view, size, pixels_per_point, scheme.hundred, margin, scheme);
}

/// [`fit`] with the device zoom allowed up to `max_device` (1 = 100 %) instead of 100 %: a small document is
/// magnified to fill the canvas, as Photoshop's Fit on Screen does.
pub fn fit_up_to(view: &mut CanvasView, size: Size, pixels_per_point: f32, max_device: f64, margin: f64, scheme: &ZoomScheme) {
    fit_rect_up_to(view, Rect::from_origin_size(Point::ORIGIN, size), size, pixels_per_point, max_device, Vec2::new(margin, margin), scheme);
}

/// Fit a part of the document on screen (an artboard, the selected layers' bounds): centre `rect`, in document
/// units, and zoom so all of it shows inside `view.rect` less `margin.x` points at the left and right and `margin.y`
/// at the top and bottom, up to the scheme's greatest zoom.
/// The zoom stays inside the range of a document of `size`; an empty `rect` zooms to that ceiling about its centre.
pub fn fit_rect(view: &mut CanvasView, rect: Rect, size: Size, pixels_per_point: f32, margin: Vec2, scheme: &ZoomScheme) {
    fit_rect_up_to(view, rect, size, pixels_per_point, scheme.max_device(), margin, scheme);
}

fn fit_rect_up_to(view: &mut CanvasView, rect: Rect, size: Size, pixels_per_point: f32, max_device: f64, margin: Vec2, scheme: &ZoomScheme) {
    let rect = rect.abs();
    let (shown_w, shown_h, sin, cos) = shown_extent(view, rect.size());
    let (bw, bh) = (shown_w * cos + shown_h * sin, shown_w * sin + shown_h * cos);
    let scale = ((f64::from(view.rect.width()) - 2.0 * margin.x) / bw).min((f64::from(view.rect.height()) - 2.0 * margin.y) / bh);
    let device = scheme.clamp((scale * f64::from(pixels_per_point)).min(max_device), size);
    view.zoom = CanvasView::points_per_unit(device, pixels_per_point);
    view.center = rect.center();
}

/// The window-canvas size at which the zoomed document fits: the canvas size less the view's
/// rect size plus the screen extent of the whole document. The difference between `canvas` and
/// the view's rect (the rulers and scrollbars) stays, so the view's rect becomes the document's
/// turned box. A document window sizes itself to it with `DocumentWindow::wrap_in`.
pub fn wrapped_canvas(view: &CanvasView, document: Size, canvas: ScreenRect) -> ScreenVec {
    canvas.size() - view.rect.size() + view.extent(Rect::from_origin_size(Point::ORIGIN, document))
}

/// Fill Screen: centre the document and zoom until it covers `view.rect`; one axis may extend
/// past the canvas.
pub fn fill(view: &mut CanvasView, size: Size, pixels_per_point: f32, scheme: &ZoomScheme) {
    let (shown_w, shown_h, sin, cos) = shown_extent(view, size);
    let (rw, rh) = (f64::from(view.rect.width()), f64::from(view.rect.height()));
    // The canvas rectangle's extent along the document's axes, which the document must cover.
    let (ex, ey) = (rw * cos + rh * sin, rw * sin + rh * cos);
    let device = scheme.clamp((ex / shown_w).max(ey / shown_h) * f64::from(pixels_per_point), size);
    view.zoom = CanvasView::points_per_unit(device, pixels_per_point);
    view.center = Point::new(size.width / 2.0, size.height / 2.0);
}

/// Set the zoom to `device_zoom` (clamped by the caller) keeping the document point under
/// `screen` where it is, or, with Zoom Clicked Point to Center (`center_on_point`), moving it to
/// the view's centre.
pub fn about(view: &mut CanvasView, screen: Pos2, device_zoom: f64, center_on_point: bool, pixels_per_point: f32) {
    let before = view.to_doc(screen);
    if center_on_point {
        view.zoom = CanvasView::points_per_unit(device_zoom, pixels_per_point);
        view.center = before;
    } else {
        anchor(view, screen, before, device_zoom, pixels_per_point);
    }
}

/// Set the zoom to `device_zoom` with the document point `doc` shown at `screen`.
pub fn anchor(view: &mut CanvasView, screen: Pos2, doc: Point, device_zoom: f64, pixels_per_point: f32) {
    view.zoom = CanvasView::points_per_unit(device_zoom, pixels_per_point);
    view.center = doc - view.unmap_vec(screen - view.rect.center());
}

/// Where a wheel or pinch zoom anchors for a pointer at `screen`: the pointer over the document,
/// else the document's nearest point (an anchor out on the pasteboard slides the image out of
/// the window after a few notches; Photoshop keeps it in view). Clamped in document space, so a
/// turned or mirrored view behaves the same.
pub fn wheel_anchor(view: &CanvasView, screen: Pos2, size: Size) -> Pos2 {
    let d = view.to_doc(screen);
    let usable = |v: f64| v.is_finite() && v > 0.0;
    if !usable(size.width) || !usable(size.height) || !d.x.is_finite() || !d.y.is_finite() {
        return screen;
    }
    let nearest = Point::new(d.x.clamp(0.0, size.width), d.y.clamp(0.0, size.height));
    if nearest == d { screen } else { view.to_screen(nearest) }
}

/// View › Zoom In (`dir` > 0) or Zoom Out (`dir` < 0): the next rung of the scheme
/// ([`ZoomScheme::step`]), the view's centre unchanged. Returns whether the zoom changed (it does
/// not at a limit).
pub fn step(view: &mut CanvasView, dir: i32, size: Size, pixels_per_point: f32, scheme: &ZoomScheme) -> bool {
    let current = device(view, pixels_per_point);
    let next = scheme.step(current, dir, size);
    let changed = next.is_finite() && (next / current - 1.0).abs() > 1e-9;
    if changed {
        view.zoom = CanvasView::points_per_unit(next, pixels_per_point);
    }
    changed
}
