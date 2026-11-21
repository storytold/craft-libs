//! `FullWindowView`: the view of a full-window canvas as plain data. The canvas view commands
//! ([`ViewCommands`](crate::canvas::ViewCommands)) act on it.

use crate::canvas::{ViewCommands, ViewTarget};
use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{CanvasView, PixelSize, ZoomScheme};
use egui::Rect;

/// Zooms closer than this are the same zoom (a round trip through points per unit and back differs in the last
/// bits).
const SAME: f64 = 1e-9;

/// The highest device zoom a fitted view reaches: a small image is magnified up to 400 % to fill the canvas.
const FIT_MAX: f64 = 4.0;

/// How a full-window canvas shows its image: fitted, or a zoom and a centre the user chose.
///
/// Plain data the workspace keeps beside its session, never in its undo history: Undo never moves the view. A new
/// view is fitted. The canvas settles it on its rect each pass (so a window resize refits a fitted view) and reports
/// the next view in its output; the zoom bar steps from the same value.
///
/// - Fitted: the whole image shows centred, magnified up to 400 % ([`ViewCommands::FitOnScreen`] with a fit ceiling of
///   400 %).
/// - A zoom (wheel, pinch, commands, the Zoom tool, the zoom bar) or a pan (Space, the middle button, the Hand tool)
///   by the user ends the fit; [`ViewCommands::FitOnScreen`] returns to it.
/// - Zooms stay inside the zoom scheme's range for the image's size; the scheme is the one the canvas last settled the
///   view with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FullWindowView {
    fit: bool,
    view: CanvasView,
    size: Size,
    pixels_per_point: f32,
    scheme: ZoomScheme,
}

impl Default for FullWindowView {
    fn default() -> Self {
        Self::new()
    }
}

impl FullWindowView {
    /// A fitted view of an image not yet shown.
    pub fn new() -> Self {
        Self {
            fit: true,
            view: CanvasView::new(Rect::ZERO, Point::ZERO, 1.0),
            size: Size::ZERO,
            pixels_per_point: 1.0,
            scheme: ZoomScheme::PHOTOSHOP,
        }
    }

    /// Whether the view fits the image: until the user zooms or pans, and after `ViewCommands::FitOnScreen`.
    pub fn is_fit(&self) -> bool {
        self.fit
    }

    /// The canvas view of the last settled pass.
    pub fn view(&self) -> CanvasView {
        self.view
    }

    /// The image's size as the last settled pass saw it.
    pub fn size(&self) -> Size {
        self.size
    }

    /// The device zoom on screen, 1 = 100 %.
    pub fn device_zoom(&self) -> f64 {
        zoom::device(&self.view, self.pixels_per_point)
    }

    /// The zoom on screen as a percentage (66.7 for 66.7 %), the fitted zoom while fitted: the title's and the zoom
    /// bar's value.
    pub fn zoom_percent(&self) -> f64 {
        self.scheme.percent(self.device_zoom())
    }

    /// The view on `rect` for an image of `size` pixels at `pixels_per_point`, zooming by `scheme`: fitted while
    /// [`is_fit`](Self::is_fit), else the zoom and centre kept. Records `size`, `scheme` and, while fitted, the zoom it
    /// fitted to.
    #[must_use]
    pub fn settled(mut self, rect: Rect, size: PixelSize, pixels_per_point: f32, scheme: &ZoomScheme) -> Self {
        self.size = Size::new(f64::from(size.width), f64::from(size.height));
        self.pixels_per_point = pixels_per_point;
        self.scheme = *scheme;
        self.view.rect = rect;
        self.refit();
        self
    }

    /// What the view commands act on: the image, fitted up to [`FIT_MAX`].
    fn target(&self) -> ViewTarget {
        ViewTarget::new(self.size).fit_ceiling(FIT_MAX)
    }

    /// Fits the view to the rect and image size the value holds, while it is fitted.
    fn refit(&mut self) {
        if self.fit && self.size.width > 0.0 && self.size.height > 0.0 {
            let target = self.target();
            ViewCommands::FitOnScreen.apply(&mut self.view, &target, None, self.pixels_per_point, &self.scheme);
        }
    }

    /// The view after the canvas reported `next` this pass (a wheel zoom, a pan, a tool); a change of the zoom or the
    /// centre ends the fit.
    #[must_use]
    pub fn took(mut self, next: CanvasView) -> Self {
        if next.zoom != self.view.zoom || next.center != self.view.center {
            self.fit = false;
        }
        self.view = next;
        self
    }

    /// Whether a zoom step in (`dir` > 0) or out would change the zoom.
    pub fn can_step(&self, dir: i32) -> bool {
        let command = if dir > 0 { ViewCommands::ZoomIn } else { ViewCommands::ZoomOut };
        command.available(&self.view, &self.target(), None, self.pixels_per_point, &self.scheme)
    }

    /// Sets the device zoom (kept inside the scheme's range) about the view's centre and ends the fit. Returns whether
    /// anything changed.
    pub fn set_device_zoom(&mut self, device: f64) -> bool {
        let device = self.scheme.clamp(device, self.size);
        let changed = self.fit || (device / self.device_zoom() - 1.0).abs() > SAME;
        let center = self.view.rect.center();
        zoom::about(&mut self.view, center, device, false, self.pixels_per_point);
        self.fit = false;
        changed
    }

    /// Applies a view command: the zoom commands (zoom in and out, fit on screen, 100 %), as
    /// [`ViewCommands::apply`] runs them on the image. The rest (fill screen, the other zooms and the rotations) leave
    /// the view as it is. Returns whether the view changed. Fit on screen refits on the rect and image size of the last
    /// settled pass, so the zoom it reports is the fitted one at once.
    pub fn apply(&mut self, command: ViewCommands) -> bool {
        match command {
            ViewCommands::ZoomIn | ViewCommands::ZoomOut => {
                let target = self.target();
                let changed = command.apply(&mut self.view, &target, None, self.pixels_per_point, &self.scheme);
                self.fit &= !changed;
                changed
            }
            ViewCommands::FitOnScreen => {
                let before = *self;
                self.fit = true;
                self.refit();
                *self != before
            }
            ViewCommands::ActualPixels => {
                let target = self.target();
                let changed = command.apply(&mut self.view, &target, None, self.pixels_per_point, &self.scheme) || self.fit;
                self.fit = false;
                changed
            }
            _ => false,
        }
    }
}
