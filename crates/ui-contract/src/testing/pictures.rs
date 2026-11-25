//! Sample content for stories in every crate: photos for tiles and previews, and the brush tips and strokes a
//! brush preview shows, so a story shows what its component shows in an app without a document or an engine.
//!
//! - [`StoryPictures`]: a key's photo painted into a thumbnail, through the one thumbnail store
//!   ([`Thumbnails`]) and a [`FakeThumbSource`] whose pictures are opaque.
//! - [`sample_picture`]: the same photo as an image, for a component that takes pixels rather than a source.
//! - [`paint_tip`], [`paint_wave`] and [`stamped_stroke`]: brush tips, a stroke drawn as a wave and a stroke
//!   stamped with round dabs, as a brush engine renders them.
//! - [`paint_contour`]: a contour curve, as a contour preset shows.
//! - [`test_card`]: colour bars over a grey ramp, as a video frame whose fit and crop show.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use craft_behavior::PixelSize;
use egui::{Color32, ColorImage, Context, Pos2, Rect, Shape, Stroke, pos2, vec2};

use crate::primitives::textures::{FakeThumbSource, Priority, ThumbFit, ThumbPaint, ThumbRequest, ThumbnailOptions, Thumbnails, fake_thumb_image};
use crate::primitives::{ContentPainterCx, StrokeWidth};

/// The photos of the stories in one context, keyed by a number: the same key shows the same picture everywhere.
#[derive(Clone)]
pub struct StoryPictures {
    thumbs: Thumbnails<u64>,
}

impl StoryPictures {
    /// The pictures stored in `ctx`.
    pub fn of(ctx: &Context) -> Self {
        Self { thumbs: Thumbnails::in_context(ctx, "craft_ui::testing::pictures", ThumbnailOptions::new()) }
    }

    /// A thumbnail painter that fills its rect with the photo of `key`, cropped to the rect's shape.
    pub fn thumb(&self, key: u64) -> impl FnOnce(&ContentPainterCx, Rect) + 'static {
        let thumbs = self.thumbs.clone();
        move |cx, rect| {
            let ctx = cx.painter.ctx().clone();
            let request = ThumbRequest::for_cell(rect.width().max(rect.height()), ctx.pixels_per_point());
            let state = thumbs.state(&ctx, &FakeThumbSource::<u64>::new().opaque(true), &key, &request, Priority::VISIBLE);
            thumbs.paint(cx, rect, &state, &ThumbPaint::default().fit(ThumbFit::Cover).checker(false));
        }
    }
}

/// The photo of `key` as an image `size` pixels wide and high: the picture [`StoryPictures::thumb`] paints.
pub fn sample_picture(key: u64, [width, height]: [u32; 2]) -> ColorImage {
    fake_thumb_image(&key, PixelSize::new(width, height))
}

/// The shape of a sample brush tip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SampleTip {
    /// A hard round dab.
    Hard,
    /// A soft round dab that fades toward its edge.
    Soft,
    /// A star outline.
    Star,
    /// A few scattered dots, as a tip sampled from an image.
    Sampled,
}

/// The tip of `tip` centred in `rect`, in `color`.
pub fn paint_tip(cx: &ContentPainterCx, rect: Rect, tip: SampleTip, color: Color32) {
    let painter = &cx.painter;
    let (centre, radius) = (rect.center(), rect.width().min(rect.height()) * 0.45);
    match tip {
        SampleTip::Hard => {
            painter.circle_filled(centre, radius, color);
        }
        SampleTip::Soft => {
            const RINGS: u8 = 6;
            for i in 0..RINGS {
                let t = f32::from(i) / f32::from(RINGS);
                painter.circle_filled(centre, radius * (1.0 - t), color.gamma_multiply(0.18));
            }
        }
        SampleTip::Star => {
            let points = (0..10)
                .map(|i| {
                    let angle = TAU * i as f32 / 10.0 - FRAC_PI_2;
                    let r = if i % 2 == 0 { radius } else { radius * 0.45 };
                    centre + vec2(angle.cos(), angle.sin()) * r
                })
                .collect();
            painter.add(Shape::closed_line(points, Stroke::new((radius * 0.15).max(1.0), color)));
        }
        SampleTip::Sampled => {
            for (x, y) in [(-1.0, -1.0), (1.0, -0.5), (0.0, 0.0), (-0.6, 0.9), (0.8, 1.0)] {
                painter.circle_filled(centre + vec2(x, y) * radius * 0.55, radius * 0.22, color);
            }
        }
    }
}

/// A stroke as one wave across `rect`, `width` as a share of the rect's height, in `color`.
pub fn paint_wave(cx: &ContentPainterCx, rect: Rect, width: f32, color: Color32) {
    let width = rect.height() * width;
    let amp = (rect.height() * 0.5 - width * 0.5).max(0.0) * 0.8;
    let points: Vec<Pos2> = (0..=24)
        .map(|i| {
            let t = i as f32 / 24.0;
            Pos2::new(rect.left() + rect.width() * t, rect.center().y - amp * (t * TAU).sin())
        })
        .collect();
    cx.painter.add(Shape::line(points, Stroke::new(width, color)));
}

/// A stroke `size` pixels wide and high: round dabs of `diameter` pixels times the pressure stamped along
/// `samples` (x and y in pixels, pressure 0 to 1), with six dabs between neighbouring samples, in `color` on
/// transparent.
pub fn stamped_stroke(size: [usize; 2], samples: &[[f64; 3]], diameter: f64, color: Color32) -> ColorImage {
    let [w, h] = size;
    let mut image = ColorImage::filled(size, Color32::TRANSPARENT);
    let mut stamp = |x: f64, y: f64, radius: f64| {
        let (x0, x1) = ((x - radius - 1.0).floor().max(0.0) as usize, ((x + radius + 1.0).ceil().max(0.0) as usize).min(w));
        let (y0, y1) = ((y - radius - 1.0).floor().max(0.0) as usize, ((y + radius + 1.0).ceil().max(0.0) as usize).min(h));
        for py in y0..y1 {
            for px in x0..x1 {
                let reach = (px as f64 + 0.5 - x).hypot(py as f64 + 0.5 - y);
                let cover = (radius + 0.5 - reach).clamp(0.0, 1.0) as f32;
                let dab = color.gamma_multiply(cover);
                let pixel = &mut image.pixels[py * w + px];
                if dab.a() > pixel.a() {
                    *pixel = dab;
                }
            }
        }
    };
    const SUBSTEPS: usize = 6;
    for pair in samples.windows(2) {
        for step in 0..=SUBSTEPS {
            let t = step as f64 / SUBSTEPS as f64;
            let at = |a: f64, b: f64| a + (b - a) * t;
            stamp(at(pair[0][0], pair[1][0]), at(pair[0][1], pair[1][1]), diameter * at(pair[0][2], pair[1][2]) / 2.0);
        }
    }
    image
}

/// Contour `n` across `rect` in the ink of the surface: a power curve for even `n`, a cosine wave for odd.
pub fn paint_contour(cx: &ContentPainterCx, rect: Rect, n: u32) {
    let area = rect.shrink(rect.width() * 0.1);
    let power = 0.6 + (n % 5) as f32 * 0.35;
    let points: Vec<Pos2> = (0..=20)
        .map(|i| {
            let t = i as f32 / 20.0;
            let y = if n.is_multiple_of(2) { t.powf(power) } else { 0.5 - 0.5 * (t * PI * (1.0 + (n % 3) as f32)).cos() };
            pos2(area.left() + t * area.width(), area.bottom() - y * area.height())
        })
        .collect();
    cx.painter.add(Shape::line(points, Stroke::new(cx.stroke_width(StrokeWidth::Thin), cx.ink())));
}

/// The size of [`test_card`] in pixels: 4:3.
pub const TEST_CARD_SIZE: [usize; 2] = [160, 120];
/// The test card's bars.
const BARS: [Color32; 8] = [
    Color32::from_rgb(0xC0, 0xC0, 0xC0),
    Color32::from_rgb(0xC0, 0xC0, 0x00),
    Color32::from_rgb(0x00, 0xC0, 0xC0),
    Color32::from_rgb(0x00, 0xC0, 0x00),
    Color32::from_rgb(0xC0, 0x00, 0xC0),
    Color32::from_rgb(0xC0, 0x00, 0x00),
    Color32::from_rgb(0x00, 0x00, 0xC0),
    Color32::from_rgb(0x10, 0x10, 0x10),
];

/// A 4:3 video test card: eight colour bars over the top two thirds, a black to white ramp under them.
pub fn test_card() -> ColorImage {
    let [width, height] = TEST_CARD_SIZE;
    let mut pixels = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            pixels.push(if y < height * 2 / 3 { BARS[x * BARS.len() / width] } else { Color32::from_gray((x * 255 / (width - 1)) as u8) });
        }
    }
    ColorImage::new(TEST_CARD_SIZE, pixels)
}
