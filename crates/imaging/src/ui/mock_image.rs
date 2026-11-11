//! [`MockImage`]: the one generated picture of craft-imaging's stories: the selection
//! and mask-view stories and every full-window editor's stories draw it.
//!
//! Original art drawn by code, so the stories show a photo-like image without a file: a gradient
//! sky, a sun with a glow, two hills, a lake, a tree, a fence and a house. 1600 × 1067 by default
//! (3:2). The house is the picture's subject: [`subject`](MockImage::subject) is its soft-edged
//! mask, the one the selection, mask-view and Select and Mask stories show.
//!
//! # For story authors
//!
//! - `MockImage::default()` is the full picture. [`MockImage::new`] draws the same scene at any
//!   size (the shapes scale with the width, the hills and the horizon with the height), so a small
//!   one is cheap for a thumbnail and a huge one costs nothing until it is read.
//! - It is a [`RasterSource`]: `RasterDisplay::new(label, &image)`, or [`story_canvas`], which
//!   shows a canvas with the picture under the story's controls.
//! - [`MockSubject`] is the subject as a [`MaskSource`] (a selection of the house): use it for the
//!   selection overlay, a Select and Mask preview or a fake mask source.
//!   [`MockImage::subject_bounds`] gives its pixels.
//! - Everything is a pure function of the coordinates: no randomness, no clock. Every method takes
//!   document pixels as `f64`; an integer pixel's centre is `+ 0.5`.
//! - Nothing here uses egui except `read`'s output type and the canvas it draws into.
//!
//! The module is behind `screen-testing`, with the stories.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use egui::{Color32, ColorImage, Ui};

use craft_behavior::kurbo::Point;
use craft_behavior::{CanvasView, PixelSize};
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::primitives::Revision;
use craft_ui::testing::StoryOutput;
use craft_ui::testing::canvas_story::{on_canvas_view, story_view};

use crate::trace::{MaskRect, MaskSource};

/// The width of the subject's soft edge, as a fraction of the picture's width: 6.4 px at the
/// default size, from fully in to fully out.
const EDGE: f64 = 0.004;

/// The generated picture as a `RasterSource`. Pixels are computed on read (no buffer), so any
/// size costs nothing until shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MockImage {
    size: [u32; 2],
    revision: u64,
}

impl Default for MockImage {
    fn default() -> Self {
        Self { size: [1600, 1067], revision: 1 }
    }
}

type Rgb = [f64; 3];

fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The signed distance from `p` to the box of centre `c` and half sizes `half`: negative inside.
fn sd_box(p: (f64, f64), c: (f64, f64), half: (f64, f64)) -> f64 {
    let (dx, dy) = ((p.0 - c.0).abs() - half.0, (p.1 - c.1).abs() - half.1);
    dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0)
}

/// The signed distance from `p` to the disc of centre `c` and radius `r`.
fn sd_disc(p: (f64, f64), c: (f64, f64), r: f64) -> f64 {
    (p.0 - c.0).hypot(p.1 - c.1) - r
}

/// The largest signed distance to the lines through the sides of a triangle given clockwise on
/// screen (y down): exact inside and beside a side, an underestimate near a corner.
fn sd_triangle(p: (f64, f64), [a, b, c]: [(f64, f64); 3]) -> f64 {
    let side = |from: (f64, f64), to: (f64, f64)| {
        let (ex, ey) = (to.0 - from.0, to.1 - from.1);
        // The outward normal of a clockwise (y down) triangle's side is `(ey, -ex)`.
        ((p.0 - from.0) * ey - (p.1 - from.1) * ex) / ex.hypot(ey)
    };
    side(a, b).max(side(b, c)).max(side(c, a))
}

/// A colour as 0..=1 channels from the 0..=255 values the art is drawn with.
fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    [f64::from(r) / 255.0, f64::from(g) / 255.0, f64::from(b) / 255.0]
}

/// The house's shapes in document pixels.
struct House {
    /// Centre and half sizes.
    wall: ((f64, f64), (f64, f64)),
    /// Apex, then the two base corners, clockwise on screen.
    roof: [(f64, f64); 3],
    door: ((f64, f64), (f64, f64)),
    window: ((f64, f64), (f64, f64)),
}

impl MockImage {
    pub(crate) fn new(size: [u32; 2]) -> Self {
        Self { size, revision: 1 }
    }

    /// The width and height as `f64`, at least 1.
    fn extent(&self) -> (f64, f64) {
        (f64::from(self.size[0].max(1)), f64::from(self.size[1].max(1)))
    }

    /// The house in document pixels. One unit `k` is the picture's width, so the house keeps its
    /// shape at any size; its base sits at 70 % of the height.
    fn house(&self) -> House {
        let (k, h) = self.extent();
        let base = (0.5 * k, 0.70 * h);
        House {
            wall: ((base.0, base.1 - 0.065 * k), (0.10 * k, 0.065 * k)),
            roof: [(base.0, base.1 - 0.23 * k), (base.0 + 0.125 * k, base.1 - 0.13 * k), (base.0 - 0.125 * k, base.1 - 0.13 * k)],
            door: ((base.0 - 0.03 * k, base.1 - 0.03 * k), (0.016 * k, 0.03 * k)),
            window: ((base.0 + 0.045 * k, base.1 - 0.075 * k), (0.022 * k, 0.02 * k)),
        }
    }

    /// The signed distance in pixels from `(x, y)` to the house's outline: negative inside, 0 on
    /// the edge. The hard shape the picture draws; [`subject`](Self::subject) softens it.
    pub(crate) fn subject_distance(&self, x: f64, y: f64) -> f64 {
        let house = self.house();
        sd_box((x, y), house.wall.0, house.wall.1).min(sd_triangle((x, y), house.roof))
    }

    /// The pixels the subject touches (coverage above 0), as a mask rectangle: the house's bounds
    /// and its soft edge.
    pub(crate) fn subject_bounds(&self) -> MaskRect {
        let (k, _) = self.extent();
        let house = self.house();
        let reach = EDGE * k;
        let left = house.roof[2].0.min(house.wall.0.0 - house.wall.1.0) - reach;
        let right = house.roof[1].0.max(house.wall.0.0 + house.wall.1.0) + reach;
        let top = house.roof[0].1 - reach;
        let bottom = house.wall.0.1 + house.wall.1.1 + reach;
        MaskRect::new(left.floor() as i32, top.floor() as i32, right.ceil() as i32, bottom.ceil() as i32)
    }

    /// The colour at document pixel `(x, y)`, sRGBA unmultiplied: sky, sun, hills, lake, shapes.
    pub(crate) fn pixel(&self, x: f64, y: f64) -> [u8; 4] {
        // new: original art for the stories.
        let ground = self.landscape(x, y);
        let edge = (0.5 - self.subject_distance(x, y)).clamp(0.0, 1.0);
        let colour = if edge > 0.0 { mix(ground, self.house_colour(x, y), edge) } else { ground };
        let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        [byte(colour[0]), byte(colour[1]), byte(colour[2]), 255]
    }

    /// Everything but the house.
    fn landscape(&self, x: f64, y: f64) -> Rgb {
        let (k, h) = self.extent();
        let (u, v) = (x / k, y / h);
        let tau = std::f64::consts::TAU;

        // Sky: deep blue at the top, pale at the horizon.
        let mut colour = mix(rgb(38, 96, 176), rgb(178, 214, 240), smoothstep(v / 0.6));

        // Sun with a glow.
        let gap = sd_disc((x, y), (0.78 * k, 0.20 * h), 0.05 * k);
        if gap < 0.0 {
            colour = rgb(255, 244, 204);
        } else {
            let glow = (1.0 - gap / (0.10 * k)).max(0.0);
            colour = mix(colour, rgb(255, 236, 180), 0.55 * glow * glow);
        }

        // Far hill, bluish; near hill, green with a little grain.
        let far = h * (0.52 + 0.04 * (u * tau * 1.3 + 0.6).sin());
        if y >= far {
            colour = mix(rgb(120, 156, 150), rgb(78, 112, 108), smoothstep((y - far) / (0.2 * h)));
        }
        let near = h * (0.64 + 0.04 * (u * tau * 0.9 + 2.1).sin() + 0.015 * (u * tau * 3.1).sin());
        if y >= near {
            let grain = 0.03 * ((x * 0.9).sin() * (y * 1.3).sin());
            let grass = mix(rgb(96, 150, 74), rgb(52, 98, 48), smoothstep((y - near) / (0.36 * h)));
            colour = [grass[0] + grain, grass[1] + grain, grass[2] + grain];
        }

        // Lake: an ellipse that mirrors the sky.
        let lake = ((x - 0.22 * k) / (0.18 * k), (y - 0.88 * h) / (0.045 * h));
        if lake.0.hypot(lake.1) < 1.0 {
            let ripple = 0.04 * (y / (0.004 * k)).sin();
            let water = mix(rgb(150, 192, 226), rgb(86, 136, 190), smoothstep((lake.1 + 1.0) / 2.0));
            colour = [water[0] + ripple, water[1] + ripple, water[2] + ripple];
        }

        // Tree: a trunk and two crowns.
        let foot = (0.14 * k, 0.72 * h);
        if sd_box((x, y), (foot.0, foot.1 - 0.04 * k), (0.006 * k, 0.04 * k)) < 0.0 {
            colour = rgb(98, 70, 46);
        }
        for (dx, dy, r, shade) in [(0.0, -0.10, 0.05, rgb(38, 94, 52)), (0.02, -0.12, 0.035, rgb(52, 116, 62))] {
            if sd_disc((x, y), (foot.0 + dx * k, foot.1 + dy * k), r * k) < 0.0 {
                colour = shade;
            }
        }

        // Fence: eleven posts and two rails.
        let fence_base = 0.76 * h;
        let (left, step) = (0.64 * k, 0.03 * k);
        let post = ((x - left) / step).round().clamp(0.0, 10.0);
        let rails = [(0.012, 0.004), (0.03, 0.004)]
            .iter()
            .any(|&(rise, half)| sd_box((x, y), (left + 5.0 * step, fence_base - rise * k), (5.0 * step + 0.003 * k, half * k)) < 0.0);
        if rails || sd_box((x, y), (left + post * step, fence_base - 0.0225 * k), (0.003 * k, 0.0225 * k)) < 0.0 {
            colour = rgb(236, 232, 222);
        }
        colour
    }

    /// The house's colours: red roof, cream wall, brown door, blue window.
    fn house_colour(&self, x: f64, y: f64) -> Rgb {
        let house = self.house();
        let p = (x, y);
        if sd_box(p, house.window.0, house.window.1) < 0.0 {
            return rgb(150, 198, 232);
        }
        if sd_box(p, house.door.0, house.door.1) < 0.0 {
            return rgb(104, 66, 40);
        }
        if sd_box(p, house.wall.0, house.wall.1) < sd_triangle(p, house.roof) { rgb(238, 224, 190) } else { rgb(168, 56, 48) }
    }

    /// The subject's coverage at document pixel `(x, y)`, 0 to 1: 1 inside the house, falling to
    /// 0 over a few pixels across its edge (a soft mask, as a refined selection has), 0 elsewhere.
    pub(crate) fn subject(&self, x: f64, y: f64) -> f32 {
        // new: the house's rectangle and roof from `pixel`'s shapes, with a smoothstep edge.
        let (k, _) = self.extent();
        smoothstep(0.5 - self.subject_distance(x, y) / (EDGE * k)) as f32
    }
}

/// One read of the picture: what [`MockImage::texels`] computes from.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ReadKey {
    size: [u32; 2],
    region: PixelRect,
    factor: u32,
    texels: [usize; 2],
}

impl MockImage {
    /// The texels of a `[columns, rows]` read of `region` at `factor`: `pixel` at each texel's centre.
    fn texels(&self, region: PixelRect, factor: u32, [columns, rows]: [usize; 2]) -> Arc<[Color32]> {
        // new: samples `pixel` at each texel's centre.
        let factor = f64::from(factor.max(1));
        let mut texels = Vec::with_capacity(columns * rows);
        for row in 0..rows {
            let y = f64::from(region.y0) + (row as f64 + 0.5) * factor;
            for column in 0..columns {
                let x = f64::from(region.x0) + (column as f64 + 0.5) * factor;
                let [r, g, b, a] = self.pixel(x, y);
                texels.push(Color32::from_rgba_unmultiplied(r, g, b, a));
            }
        }
        texels.into()
    }
}

impl RasterSource for MockImage {
    fn size(&self) -> PixelSize {
        PixelSize::new(self.size[0], self.size[1])
    }

    fn revision(&self) -> Revision {
        Revision::new(self.revision)
    }

    fn damage(&self, since: Revision) -> Option<PixelRect> {
        (since == self.revision()).then(PixelRect::default)
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        // Every fixture of a trial run reads the same tiles anew, and computing them took most of the stories' trial
        // time, so the texels of each read are kept for the process. They depend on the size, not on the revision.
        static READS: Mutex<Option<HashMap<ReadKey, Arc<[Color32]>>>> = Mutex::new(None);
        let key = ReadKey { size: self.size, region, factor, texels: out.size };
        let cached = READS.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert_default().get(&key).cloned();
        let texels = cached.unwrap_or_else(|| {
            let texels = self.texels(region, factor, out.size);
            READS.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert_default().insert(key, texels.clone());
            texels
        });
        out.pixels.copy_from_slice(&texels);
    }
}

/// The picture's subject as a selection: a [`MaskSource`] over [`MockImage::subject`], with the
/// picture's size and revision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MockSubject(pub(crate) MockImage);

impl MaskSource for MockSubject {
    fn size(&self) -> [u32; 2] {
        self.0.size
    }

    fn revision(&self) -> u64 {
        self.0.revision
    }

    fn content_bounds(&self) -> MaskRect {
        self.0.subject_bounds()
    }

    fn read_row(&self, y: i32, x0: i32, step: u32, out: &mut [f32]) {
        let (y, step) = (f64::from(y) + 0.5, f64::from(step));
        for (i, value) in out.iter_mut().enumerate() {
            *value = self.0.subject(f64::from(x0) + 0.5 + i as f64 * step, y);
        }
    }
}

/// How a story canvas shows its picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum StoryView {
    /// The whole picture fits the canvas.
    Fit,
    /// `zoom` percent (device pixels per picture pixel), the picture's middle at the canvas's middle.
    Zoom(f64),
    /// `zoom` percent, `centre` (picture pixels) at the canvas's middle.
    At { zoom: f64, centre: Point },
}

/// A story canvas over the stage: `image` (the default picture's size without one) under `view`, then `controls` called with the
/// canvas's rect to show the controls of the story over it.
pub(crate) fn story_canvas(
    ui: &mut Ui,
    image: Option<&dyn RasterSource>,
    view: StoryView,
    mut controls: impl FnMut(&mut Ui, egui::Rect) -> StoryOutput,
) -> StoryOutput {
    let size = image.map_or_else(|| MockImage::default().size(), |image| image.size());
    on_canvas_view(ui, |at, ppp| view.of(at, size, ppp), image, |ui, rect, _| controls(ui, rect))
}

impl StoryView {
    /// The view of the canvas rect `at` over a picture of `size` for `pixels_per_point` device pixels per point.
    pub(crate) fn of(self, at: egui::Rect, size: PixelSize, pixels_per_point: f32) -> CanvasView {
        let (width, height) = (f64::from(size.width.max(1)), f64::from(size.height.max(1)));
        let middle = Point::new(width / 2.0, height / 2.0);
        match self {
            StoryView::Fit => {
                let fit = (f64::from(at.width()) / width).min(f64::from(at.height()) / height);
                CanvasView::new(at, middle, fit)
            }
            StoryView::Zoom(percent) => story_view(at, pixels_per_point, middle, percent),
            StoryView::At { zoom, centre } => story_view(at, pixels_per_point, centre, zoom),
        }
    }
}
