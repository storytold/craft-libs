//! [`CanvasView`]: the mapping between a document and the screen. The document is f64 `kurbo` space; the
//! screen side is `emath` f32 points.

use super::CanvasTransform;
use crate::cursor::CursorIcon;
use emath::{Pos2, Rect as ScreenRect, Vec2 as ScreenVec, pos2, vec2};
use kurbo::{Affine, Point, Rect, Vec2};

/// How a canvas shows its document: where, how large, turned and mirrored how. Apps convert their
/// own view type into it once per frame; overlays map their document geometry through it.
///
/// The document is turned by [`rotation`](Self::rotation) about [`center`](Self::center), after
/// [`aspect`](Self::aspect) stretches it along its x axis and before
/// [`flip_x`](Self::flip_x) mirrors the result, PhotoCraft's `ViewXform`. DesignCraft's quarter
/// turns are `rotation = 90 × q`; VectorCraft's radians are converted to degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CanvasView {
    /// The canvas on screen, inside the rulers.
    pub rect: ScreenRect,
    /// The document point drawn at `rect.center()`.
    pub center: Point,
    /// Screen points per document unit. In PhotoCraft's model this is the view zoom over the
    /// pixels per point, so 100 % shows one device pixel per document pixel.
    pub zoom: f64,
    /// Degrees, clockwise on screen.
    pub rotation: f64,
    /// The view is mirrored left-right; the document is not.
    pub flip_x: bool,
    /// Width over height of a document unit on screen (PhotoCraft's pixel aspect); 1 is square.
    pub aspect: f64,
}

impl CanvasView {
    /// An upright, unmirrored, square-pixel view of `rect` showing `center` at `zoom`.
    pub fn new(rect: ScreenRect, center: Point, zoom: f64) -> Self {
        Self { rect, center, zoom, rotation: 0.0, flip_x: false, aspect: 1.0 }
    }

    /// The same view turned `degrees` clockwise.
    #[must_use]
    pub const fn rotation(mut self, degrees: f64) -> Self {
        self.rotation = degrees;
        self
    }

    /// The same view mirrored left-right (or not).
    #[must_use]
    pub const fn flip_x(mut self, flip: bool) -> Self {
        self.flip_x = flip;
        self
    }

    /// The same view with a document unit `aspect` times as wide as it is tall.
    #[must_use]
    pub const fn aspect(mut self, aspect: f64) -> Self {
        self.aspect = aspect;
        self
    }

    /// `zoom` for a view that shows `device_zoom` device pixels per document unit at
    /// `pixels_per_point`: 1 shows one document pixel per device pixel at any interface scale. A scale that is not a
    /// positive number counts as 1.
    pub fn points_per_unit(device_zoom: f64, pixels_per_point: f32) -> f64 {
        let ppp = if pixels_per_point.is_finite() && pixels_per_point > 0.0 { pixels_per_point } else { 1.0 };
        device_zoom / f64::from(ppp)
    }

    /// `aspect`, or 1 when it is not a positive number.
    fn sx(&self) -> f64 {
        if self.aspect.is_finite() && self.aspect > 0.0 { self.aspect } else { 1.0 }
    }

    /// Cosine and sine of the rotation, exact at quarter turns so an upright or quarter-turned
    /// view maps axis-aligned lines to axis-aligned lines.
    fn sincos(&self) -> (f64, f64) {
        let turns = self.rotation / 90.0;
        if (turns - turns.round()).abs() < 1e-9 {
            match (turns.round() as i64).rem_euclid(4) {
                0 => (1.0, 0.0),
                1 => (0.0, 1.0),
                2 => (-1.0, 0.0),
                _ => (0.0, -1.0),
            }
        } else {
            let (s, c) = self.rotation.to_radians().sin_cos();
            (c, s)
        }
    }

    fn screen_center(&self) -> Vec2 {
        let c = self.rect.center();
        Vec2::new(f64::from(c.x), f64::from(c.y))
    }

    /// The linear part of [`affine`](Self::affine): stretch, rotate, zoom, flip.
    fn linear(&self) -> [f64; 4] {
        let (c, s) = self.sincos();
        let (sx, z) = (self.sx(), self.zoom);
        let f = if self.flip_x { -1.0 } else { 1.0 };
        // Columns of the matrix: the images of the document's x and y unit vectors.
        [f * c * sx * z, s * sx * z, -f * s * z, c * z]
    }

    /// Document to screen points.
    pub fn affine(&self) -> Affine {
        let [a, b, c, d] = self.linear();
        let m = Affine::new([a, b, c, d, 0.0, 0.0]);
        Affine::translate(self.screen_center()) * m * Affine::translate(-self.center.to_vec2())
    }

    /// Where the view draws document point `p`.
    pub fn to_screen(&self, p: Point) -> Pos2 {
        let s = self.affine() * p;
        pos2(s.x as f32, s.y as f32)
    }

    /// The document point under screen position `p`.
    pub fn to_doc(&self, p: Pos2) -> Point {
        let d = self.unmap_vec(p - self.rect.center());
        self.center + d
    }

    /// A document vector as the view draws it: stretched, turned, zoomed and mirrored.
    pub fn map_vec(&self, v: Vec2) -> ScreenVec {
        let [a, b, c, d] = self.linear();
        vec2((a * v.x + c * v.y) as f32, (b * v.x + d * v.y) as f32)
    }

    /// The document vector a screen vector shows: the inverse of [`map_vec`](Self::map_vec).
    pub fn unmap_vec(&self, v: ScreenVec) -> Vec2 {
        let (c, s) = self.sincos();
        let (x, y) = (f64::from(v.x) / self.zoom, f64::from(v.y) / self.zoom);
        let x = if self.flip_x { -x } else { x };
        Vec2::new((x * c + y * s) / self.sx(), -x * s + y * c)
    }

    /// Screen points per document unit along the document's x axis: `zoom × aspect`.
    pub fn zoom_x(&self) -> f64 {
        self.zoom * self.sx()
    }

    /// The screen corners of document rect `r`, in order round it: (x0, y0), (x1, y0), (x1, y1),
    /// (x0, y1).
    pub fn quad(&self, r: Rect) -> [Pos2; 4] {
        [
            self.to_screen(Point::new(r.x0, r.y0)),
            self.to_screen(Point::new(r.x1, r.y0)),
            self.to_screen(Point::new(r.x1, r.y1)),
            self.to_screen(Point::new(r.x0, r.y1)),
        ]
    }

    /// The document bounds of the screen rect: every document point the view can show.
    pub fn visible(&self) -> Rect {
        let r = self.rect;
        let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()].map(|p| self.to_doc(p));
        corners.iter().skip(1).fold(Rect::from_points(corners[0], corners[0]), |acc, &p| acc.union_pt(p))
    }

    /// A screen distance in document units along the document's y axis (divide by
    /// [`zoom_x`](Self::zoom_x) for its x axis).
    pub fn doc_len(&self, points: f32) -> f64 {
        f64::from(points) / self.zoom
    }

    /// The transform a canvas body is shown through when this view only scales and moves the document: unturned,
    /// unmirrored and square. `None` for any other view; its controls register parts in screen space through
    /// [`to_screen`](Self::to_screen) instead.
    pub fn layer_transform(&self) -> Option<CanvasTransform> {
        let (cos, sin) = self.sincos();
        let upright = cos == 1.0 && sin == 0.0 && !self.flip_x && self.sx() == 1.0;
        if !upright {
            return None;
        }
        let zoom = self.zoom as f32;
        let center = self.rect.center();
        let offset = vec2(center.x - self.center.x as f32 * zoom, center.y - self.center.y as f32 * zoom);
        Some(CanvasTransform::new(zoom, offset))
    }

    /// Whether the rotation is a multiple of 90°, so document axes are screen axes.
    pub fn axis_aligned(&self) -> bool {
        let turns = self.rotation / 90.0;
        (turns - turns.round()).abs() < 1e-9
    }

    /// The view without its rotation, moved so it draws `zero` where this view does: the same
    /// zoom, flip and aspect, so ticks measured with it never stretch. Rulers measure with it and
    /// stay level on a rotated canvas, 0 under the zero point (PhotoCraft's `ruler_xform`, #3233).
    pub fn level(&self, zero: Point) -> CanvasView {
        let level = CanvasView { rotation: 0.0, ..*self };
        // The level view only scales, so moving it by `shift` on screen moves every point by it.
        let shift = self.to_screen(zero) - level.to_screen(zero);
        let center = level.center - level.unmap_vec(shift);
        if center.x.is_finite() && center.y.is_finite() { CanvasView { center, ..level } } else { level }
    }
}

/// The resize arrow for moving along document direction `doc_dir`, as `view` shows it (its
/// rotation, flip and aspect): the nearest of the four arrows. Guides take the arrow across their
/// line, box edges the arrow across their edge (PhotoCraft's `tool_cursor::resize`).
pub fn resize_cursor(view: &CanvasView, doc_dir: Vec2) -> CursorIcon {
    let s = view.map_vec(doc_dir);
    let angle = f64::from(s.y).atan2(f64::from(s.x)).rem_euclid(std::f64::consts::PI);
    match (angle / std::f64::consts::FRAC_PI_4).round() as u8 % 4 {
        0 => CursorIcon::ResizeHorizontal,
        1 => CursorIcon::ResizeNwSe,
        2 => CursorIcon::ResizeVertical,
        _ => CursorIcon::ResizeNeSw,
    }
}
