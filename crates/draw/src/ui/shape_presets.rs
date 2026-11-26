//! The shape menu: the preset glyphs, their words and [`ShapeMenu`].

use crate::geometry::presets::{PRESETS, ShapePreset};
use craft_theme::Opacity;
use craft_ui::contract::{Props as _, Show as _, ShowContainer as _};
use craft_ui::display::Divider;
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::primitives::{Glyph, IconSpec};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::UiTokens;
use egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, Ui, pos2};
use kurbo::{BezPath, PathEl};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The error bound, in points, when the glyph's curves are flattened.
const FLATTEN_TOLERANCE: f64 = 0.1;

/// The widest a stroke is, as a share of the glyph's shorter side.
const STROKE_SHARE: f32 = 0.25;

craft_ui::words! {
    /// The labels of the shape presets, English by default (upstream's).
    pub struct ShapePresetWords {
        /// The accessible name of the shape menu.
        menu: "shapePreset.menu" = "Shapes";
        /// The Rectangle preset's label.
        rectangle: "shapePreset.rectangle" = "Rectangle";
        /// The Rounded Rectangle preset's label.
        rounded_rectangle: "shapePreset.roundedRectangle" = "Rounded Rectangle";
        /// The Oval preset's label.
        ellipse: "shapePreset.ellipse" = "Oval";
        /// The Triangle preset's label.
        triangle: "shapePreset.triangle" = "Triangle";
        /// The Diamond preset's label.
        diamond: "shapePreset.diamond" = "Diamond";
        /// The Line preset's label.
        line: "shapePreset.line" = "Line";
        /// The Arrow preset's label.
        arrow: "shapePreset.arrow" = "Arrow";
        /// The Star preset's label.
        star: "shapePreset.star" = "Star";
        /// The Heart preset's label.
        heart: "shapePreset.heart" = "Heart";
        /// The Text Box preset's label.
        text_box: "shapePreset.textBox" = "Text Box";
        /// The Straight Connector preset's label.
        straight_connector: "shapePreset.straightConnector" = "Straight Connector";
        /// The Straight Arrow Connector preset's label.
        straight_arrow_connector: "shapePreset.straightArrowConnector" = "Straight Arrow Connector";
        /// The Elbow Connector preset's label.
        elbow_connector: "shapePreset.elbowConnector" = "Elbow Connector";
        /// The Elbow Arrow Connector preset's label.
        elbow_arrow_connector: "shapePreset.elbowArrowConnector" = "Elbow Arrow Connector";
        /// The Curved Connector preset's label.
        curved_connector: "shapePreset.curvedConnector" = "Curved Connector";
        /// The Curved Arrow Connector preset's label.
        curved_arrow_connector: "shapePreset.curvedArrowConnector" = "Curved Arrow Connector";
    }
}

impl ShapePreset {
    /// The preset's label in `words`.
    pub fn label(self, words: &ShapePresetWords) -> &Label {
        match self {
            ShapePreset::Rectangle => &words.rectangle,
            ShapePreset::RoundedRectangle => &words.rounded_rectangle,
            ShapePreset::Ellipse => &words.ellipse,
            ShapePreset::Triangle => &words.triangle,
            ShapePreset::Diamond => &words.diamond,
            ShapePreset::Line => &words.line,
            ShapePreset::Arrow => &words.arrow,
            ShapePreset::Star => &words.star,
            ShapePreset::Heart => &words.heart,
            ShapePreset::TextBox => &words.text_box,
            ShapePreset::StraightConnector => &words.straight_connector,
            ShapePreset::StraightArrowConnector => &words.straight_arrow_connector,
            ShapePreset::ElbowConnector => &words.elbow_connector,
            ShapePreset::ElbowArrowConnector => &words.elbow_arrow_connector,
            ShapePreset::CurvedConnector => &words.curved_connector,
            ShapePreset::CurvedArrowConnector => &words.curved_arrow_connector,
        }
    }
}

/// Presets never mirror: each glyph shows the document shape its tool creates, and document geometry does not follow
/// the reading direction.
impl Glyph for ShapePreset {
    fn paint(&self, painter: &Painter, tokens: &UiTokens, rect: Rect, color: Color32, _mirrored: bool) {
        if !rect.is_finite() || rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        let outline = self.outline(kurbo::Rect::new(f64::from(rect.min.x), f64::from(rect.min.y), f64::from(rect.max.x), f64::from(rect.max.y)));
        // The outline stays inside the glyph's square, and so does its stroke.
        let painter = painter.with_clip_rect(painter.clip_rect().intersect(rect));
        let thin = tokens.stroke.thin;
        let width = if thin.is_finite() { thin.clamp(0.0, rect.width().min(rect.height()) * STROKE_SHARE) } else { 0.0 };
        let stroke = Stroke::new(width, color);
        if let Some(fill) = &outline.fill {
            for (points, _) in polylines(fill) {
                fill_fan(&painter, rect.center(), points, color.gamma_multiply(Opacity::TINT));
            }
        }
        for path in &outline.strokes {
            for (points, closed) in polylines(path) {
                painter.add(if closed { Shape::closed_line(points, stroke) } else { Shape::line(points, stroke) });
            }
        }
    }
}

/// The subpaths of `path` as points, each with whether it is closed.
fn polylines(path: &BezPath) -> Vec<(Vec<Pos2>, bool)> {
    fn finish(found: &mut Vec<(Vec<Pos2>, bool)>, current: &mut Vec<Pos2>, closed: bool) {
        let points = std::mem::take(current);
        if points.len() >= 2 {
            found.push((points, closed));
        }
    }
    let at = |p: kurbo::Point| pos2(p.x as f32, p.y as f32);
    let mut found = Vec::new();
    let mut current = Vec::new();
    kurbo::flatten(path.iter(), FLATTEN_TOLERANCE, |element| match element {
        PathEl::MoveTo(p) => {
            finish(&mut found, &mut current, false);
            current.push(at(p));
        }
        PathEl::LineTo(p) => current.push(at(p)),
        PathEl::ClosePath => finish(&mut found, &mut current, true),
        PathEl::QuadTo(..) | PathEl::CurveTo(..) => {}
    });
    finish(&mut found, &mut current, false);
    found
}

/// Fills the contour `points` by a triangle fan from `centre`. The preset contours are star-shaped about their
/// glyph's centre (it lies in every edge's inner half-plane), so the fan covers their concavities, which a convex
/// polygon fill would not. The triangles wind the same way whatever way the contour runs.
fn fill_fan(painter: &Painter, centre: Pos2, mut points: Vec<Pos2>, fill: Color32) {
    if points.len() < 3 || fill == Color32::TRANSPARENT {
        return;
    }
    let area: f32 = points.iter().zip(points.iter().cycle().skip(1)).map(|(a, b)| a.x * b.y - b.x * a.y).sum();
    if area < 0.0 {
        points.reverse();
    }
    let mut mesh = Mesh::default();
    mesh.colored_vertex(centre, fill);
    for &point in &points {
        mesh.colored_vertex(point, fill);
    }
    for index in 0..points.len() {
        mesh.add_triangle(0, 1 + index as u32, 1 + ((index + 1) % points.len()) as u32);
    }
    painter.add(Shape::mesh(mesh));
}

/// The shape menu: the supported presets as menu items with the preset glyph as icon, a separator before the first
/// connector.
#[must_use = "show the menu with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct ShapeMenu<'a> {
    supported: Box<dyn Fn(ShapePreset) -> bool + 'a>,
}

impl<'a> ShapeMenu<'a> {
    /// The menu of the presets `supported` accepts.
    pub fn new(supported: impl Fn(ShapePreset) -> bool + 'a) -> Self {
        ShapeMenu { supported: Box::new(supported) }
    }

    /// Shows the menu and reports the picked preset. A row's identity is its preset's key, so presets with the same
    /// translated label stay apart.
    pub fn show(self, ui: &mut Ui) -> Option<ShapePreset> {
        let ShapeMenu { supported } = self;
        let words = Env::of(ui).words::<ShapePresetWords>();
        Menu::inline(words.menu.clone())
            .show(ui, |ui, _| {
                let mut picked = None;
                let mut previous_connector = false;
                let mut any = false;
                for preset in PRESETS {
                    if !supported(*preset) {
                        continue;
                    }
                    if any && preset.is_connector() && !previous_connector {
                        Divider::horizontal().show(ui);
                    }
                    previous_connector = preset.is_connector();
                    any = true;
                    let row = MenuItem::new(preset.label(&words).clone()).icon(IconSpec::glyph(preset)).id_salt(preset.key());
                    if row.show(ui).ran {
                        picked = Some(*preset);
                    }
                }
                picked
            })
            .inner
            .flatten()
    }
}
