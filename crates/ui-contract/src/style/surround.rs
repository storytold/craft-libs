//! The canvas surround: the fill around a document and the theme's pattern over it.

use super::UiTokens;
use super::paint::color as rgba_color;
use crate::primitives::textures::UNTINTED;
use crate::primitives::{CheckerArea, TextureCache, fill_polygon};
use egui::epaint::{RectShape, Vertex};
use egui::{Color32, ColorImage, Mesh, Pos2, Rect, Shape, TextureFilter, TextureOptions, TextureWrapMode};

/// The side of the pattern's tile, in points: one dot per tile.
const TILE: f32 = 22.0;

/// The dot's four texels at the tile centre and their share of the pattern colour's alpha, the dot an antialiased
/// point slightly off the texel grid.
const DOT: [((usize, usize), u8); 4] = [((10, 10), 255), ((11, 10), 110), ((10, 11), 110), ((11, 11), 60)];

/// How the surround around a canvas looks: its fill and the pattern drawn over it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct SurroundLook {
    /// The surround's fill.
    pub fill: Color32,
    /// The pattern's colour; `None` draws no pattern.
    pub pattern: Option<Color32>,
}

impl SurroundLook {
    /// The surround of the theme `tokens` resolve from: the `surface.canvas` fill, and `surface.canvas_pattern` as
    /// the pattern where the theme sets one. A theme whose pattern equals its fill has none, so no texture is made
    /// that would paint nothing (#4542).
    pub fn of(tokens: &UiTokens) -> Self {
        let surface = &tokens.color.surface;
        let pattern = (surface.canvas_pattern != surface.canvas).then(|| rgba_color(surface.canvas_pattern));
        Self { fill: rgba_color(surface.canvas), pattern }
    }

    /// The same look with the pattern kept when `pattern_on`, else dropped. Callers pass the app's preference,
    /// `EnvView::canvas_pattern`, which a style cannot read.
    #[must_use]
    pub fn shown(self, pattern_on: bool) -> Self {
        Self { pattern: self.pattern.filter(|_| pattern_on), ..self }
    }
}

/// Paints the surround `look` over `area`: the fill, and with a pattern one textured shape of the dot tile over it.
/// The tile is laid in screen space from the origin, so the dots hold still while the view scrolls or turns. A rect takes its corners; a quad (a piece of a turned pasteboard) is filled as `Draw` fills a polygon.
pub fn paint_surround(painter: &egui::Painter, area: CheckerArea, look: &SurroundLook) {
    match area {
        CheckerArea::Rect(rect, radius) => {
            painter.add(Shape::Rect(RectShape::filled(rect, radius, look.fill)));
        }
        CheckerArea::Quad(corners) => {
            let feather = painter.ctx().tessellation_options(|options| {
                if options.feathering { options.feathering_size_in_pixels / painter.pixels_per_point() } else { 0.0 }
            });
            let mesh = fill_polygon(&corners, look.fill, feather);
            if !mesh.is_empty() {
                painter.add(Shape::mesh(mesh));
            }
        }
    }
    let Some(pattern) = look.pattern else { return };
    let texture = dot_texture(painter.ctx(), pattern);
    let uv = |p: Pos2| (p.to_vec2() / TILE).to_pos2();
    let shape = match area {
        CheckerArea::Rect(rect, radius) => {
            Shape::Rect(RectShape::filled(rect, radius, UNTINTED).with_texture(texture, Rect::from_min_max(uv(rect.min), uv(rect.max))))
        }
        CheckerArea::Quad(corners) => {
            let mut mesh = Mesh::with_texture(texture);
            mesh.vertices.extend(corners.iter().map(|&pos| Vertex { pos, uv: uv(pos), color: UNTINTED }));
            mesh.indices.extend([0, 1, 2, 0, 2, 3]);
            Shape::mesh(mesh)
        }
    };
    painter.add(shape);
}

/// The repeating tile of one `pattern` dot, from the runtime's texture cache: one texture per pattern colour.
fn dot_texture(ctx: &egui::Context, pattern: Color32) -> egui::TextureId {
    let options = TextureOptions {
        magnification: TextureFilter::Linear,
        minification: TextureFilter::Linear,
        wrap_mode: TextureWrapMode::Repeat,
        mipmap_mode: None,
    };
    TextureCache::texture_with(ctx, ("craft-ui-surround", pattern), options, || {
        let side = TILE as usize;
        let [r, g, b, a] = pattern.to_srgba_unmultiplied();
        let mut pixels = vec![Color32::TRANSPARENT; side * side];
        for ((x, y), share) in DOT {
            let alpha = (u16::from(a) * u16::from(share) / 255) as u8;
            if let Some(pixel) = pixels.get_mut(y * side + x) {
                *pixel = Color32::from_rgba_unmultiplied(r, g, b, alpha);
            }
        }
        ColorImage::new([side, side], pixels)
    })
}
