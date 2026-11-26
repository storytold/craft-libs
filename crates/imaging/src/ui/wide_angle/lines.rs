//! `ConstraintLines`: the constraints of an Adaptive Wide Angle session as lines on the canvas, a canvas control.
//!
//! Each constraint is one focusable part of the control's group. The pointer picks a constraint (and a secondary
//! click on its curve asks for the context menu); the arrow keys move the focused constraint one document pixel
//! (Shift ten) and Delete or Backspace removes it. The edits come back in the output: the session is the app's.

use craft_behavior::kurbo::Point;
use craft_behavior::{CanvasView, Granularity, HitShape, Intent, Metrics, NavShape, PartKey, Roving, Sense};
use craft_ui::canvas::{canvas_space, context_click};
use craft_ui::contract::{
    Access, Common, Component, Cx, Focus as _, MeasureCx, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props as _, Role, WordSetCx as _,
};
use craft_ui::primitives::{FocusKind, FocusRing, PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::KeyUse;
use craft_ui::style::{ComponentStyle, FocusLook, StyleColor, StyleCx};
use egui::{Pos2, Rect, Response, Vec2};

use super::session::{ConstraintOrientation, WideAngleConstraintEdit, WideAngleGeometry, WideAngleSession};
use super::words::WideAngleWords;

/// Segments of the curve a constraint is drawn as (PhotoCraft's `Camera::arc(a, b, 24)`).
const ARC_SEGMENTS: usize = 24;

/// The step of an arrow key in document pixels; Shift takes `NUDGE_FAR`.
const NUDGE: f64 = 1.0;

/// The step of an arrow key with Shift held, in document pixels.
const NUDGE_FAR: f64 = 10.0;

/// The part of one constraint: `CONSTRAINT.at(index)`.
const CONSTRAINT: PartKey = PartKey::new("constraint");

/// The look of the constraint lines: a halo under each line so it reads on any image, one colour per orientation.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ConstraintLinesStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// A Free constraint.
    free: StyleColor,
    /// A Horizontal constraint.
    horizontal: StyleColor,
    /// A Vertical constraint.
    vertical: StyleColor,
    /// The constraint being drawn.
    drawing: StyleColor,
    /// The halo under every line and end dot.
    halo: StyleColor,
    /// The radius of an end dot.
    end: f32,
    focus: FocusLook,
}

/// What the constraint lines report.
#[derive(Debug)]
#[non_exhaustive]
pub struct ConstraintLinesOutput {
    /// The control's response.
    pub response: Response,
    /// The keyboard moved or deleted the focused constraint; the app applies it to its session.
    pub edit: Option<WideAngleConstraintEdit>,
    /// The constraint whose curve was clicked with the secondary button, or that asked for its menu from the
    /// keyboard.
    pub context: Option<usize>,
}

impl Outcome for ConstraintLinesOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The constraints of `session`, drawn as the lens curves of their scene lines, with a constraint being drawn.
///
/// - The canvas is in the pixels of the picture it shows (`canvas`), the constraints in document pixels
///   (`document`); [`picture`](Self::picture) says both so a proxy of the layer lines up.
/// - [`corrected`](Self::corrected) maps the curves through the solved mesh, for the corrected preview.
/// - The group is named by the label; a constraint is named by its number and orientation.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control, content_painter)]
pub struct ConstraintLines<'a> {
    #[common]
    common: Common,
    session: &'a WideAngleSession,
    geometry: &'a dyn WideAngleGeometry,
    /// Whether the canvas shows the corrected preview: the curves go through the solved mesh.
    #[prop]
    corrected: bool,
    /// Document pixels per canvas unit, per axis.
    scale: [f64; 2],
    /// The constraint being drawn, in canvas units.
    #[prop]
    draft: Option<[Point; 2]>,
    edit: Option<WideAngleConstraintEdit>,
    context: Option<usize>,
}

impl<'a> ConstraintLines<'a> {
    /// The constraints of `session` named `label`, curved by `geometry`.
    pub fn new(label: impl Into<Label>, session: &'a WideAngleSession, geometry: &'a dyn WideAngleGeometry) -> Self {
        ConstraintLines { common: Common::new(), session, geometry, corrected: false, scale: [1.0, 1.0], draft: None, edit: None, context: None }
            .label(label)
    }

    /// The canvas shows a picture of `canvas` pixels for a document of `document` pixels.
    #[must_use]
    pub fn picture(mut self, canvas: [u32; 2], document: [u32; 2]) -> Self {
        let ratio = |document: u32, canvas: u32| if canvas == 0 { 1.0 } else { f64::from(document) / f64::from(canvas) };
        self.scale = [ratio(document[0], canvas[0]), ratio(document[1], canvas[1])];
        self
    }

    /// The curve of the scene line `a` to `b` in canvas units: the lens arc, through the solved mesh while the
    /// corrected preview shows (the unmapped arc while no solve is ready).
    fn curve(&self, a: [f64; 2], b: [f64; 2]) -> Vec<Point> {
        let arc = self.geometry.arc(self.session, a, b, ARC_SEGMENTS);
        let mapped = self.corrected.then(|| arc.iter().map(|&q| self.geometry.corrected(self.session, q)).collect::<Option<Vec<_>>>()).flatten();
        mapped.unwrap_or(arc).into_iter().map(|q| Point::new(q.x / self.scale[0], q.y / self.scale[1])).collect()
    }

    fn screen(&self, view: &CanvasView, curve: Vec<Point>) -> Vec<Pos2> {
        curve.into_iter().map(|q| view.to_screen(q)).collect()
    }
}

impl Component for ConstraintLines<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::CANVAS.with_moves().with_deletes().with_chrome_key(false);
    type Style = ConstraintLinesStyle;
    type Output = ConstraintLinesOutput;

    fn style(cx: &StyleCx<'_>, _: ConstraintLinesStyleKey) -> ConstraintLinesStyle {
        let tokens = cx.tokens;
        ConstraintLinesStyle {
            metrics: Metrics::new(),
            free: cx.color(tokens.color.accent.solid),
            horizontal: cx.color(tokens.color.status.warn.fg),
            vertical: cx.color(tokens.color.status.bad.fg),
            drawing: cx.color(tokens.color.accent.soft),
            halo: cx.color(tokens.color.surface.panel),
            end: tokens.size.handle.sm,
            focus: cx.focus_look(),
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    /// One focusable part per constraint, on its curve, in one roving group in constraint order.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        for (index, constraint) in self.session.constraints.iter().enumerate() {
            let points = self.screen(&space.view, self.curve(constraint.a, constraint.b));
            cx.part(CONSTRAINT.at(index as u64), HitShape::polyline(points, false))
                .sense(Sense::CLICK | Sense::CONTEXT_CLICK | Sense::TAKE_FOCUS)
                .focusable();
        }
        if !self.session.constraints.is_empty() {
            cx.roving(Roving::new(NavShape::Flat).page(Some(1)));
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let count = self.session.constraints.len();
        for index in 0..count {
            let part = CONSTRAINT.at(index as u64);
            let Some(result) = cx.part_if_declared(part) else { continue };
            if context_click(&result.press).is_some() || (cx.active_part() == Some(part) && cx.intents().take(Intent::ContextMenu)) {
                self.context = Some(index);
            }
        }
        let Some(index) = cx.active_part().filter(|part| part.key == CONSTRAINT).and_then(|part| part.index).map(|i| i as usize) else { return };
        if index >= count {
            return;
        }
        if let Some((direction, mods)) = cx.intents().take_nudge() {
            let step = if mods.shift { NUDGE_FAR } else { NUDGE };
            let view = canvas_space(&cx.env).map(|space| space.view);
            let by = view.map_or(Vec2::new(direction.x, direction.y), |view| {
                let doc = view.to_doc(Pos2::ZERO + direction) - view.to_doc(Pos2::ZERO);
                let length = doc.hypot();
                if length > 0.0 { Vec2::new((doc.x / length) as f32, (doc.y / length) as f32) } else { direction }
            });
            let by = [f64::from(by.x) * step * self.scale[0], f64::from(by.y) * step * self.scale[1]];
            self.edit = Some(WideAngleConstraintEdit::Move { index, by });
        }
        let forward = cx.intents().take_exact(&Intent::delete(true, Granularity::Unit));
        let backward = cx.intents().take_exact(&Intent::delete(false, Granularity::Unit));
        if forward || backward {
            self.edit = Some(WideAngleConstraintEdit::Delete(index));
            if count > 1 {
                cx.focus_part(CONSTRAINT.at(index.min(count - 2) as u64));
            }
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let style = &cx.style;
        let draw = ui.draw();
        let tint = |orientation: ConstraintOrientation| match orientation {
            ConstraintOrientation::Free => style.free,
            ConstraintOrientation::Horizontal => style.horizontal,
            ConstraintOrientation::Vertical => style.vertical,
        };
        let focused = cx.focus_visible().then(|| cx.active_part()).flatten().filter(|part| part.key == CONSTRAINT).and_then(|part| part.index);
        let mut ring = None;
        for (index, constraint) in self.session.constraints.iter().enumerate() {
            let points = self.screen(&space.view, self.curve(constraint.a, constraint.b));
            let color = tint(constraint.orientation);
            let path = PaintShape::Path(points.clone());
            draw.stroke(&path, StrokeWidth::Thick, style.halo);
            draw.stroke(&path, StrokeWidth::Medium, color);
            for end in [points.first(), points.last()].into_iter().flatten() {
                let dot = PaintShape::Circle(*end, style.end);
                draw.fill(&PaintShape::Circle(*end, style.end), style.halo);
                draw.fill(&dot, color);
            }
            if focused == Some(index as u64) {
                ring = Some(Rect::from_points(&points));
            }
        }
        if let Some([a, b]) = self.draft {
            let (a, b) = ([a.x * self.scale[0], a.y * self.scale[1]], [b.x * self.scale[0], b.y * self.scale[1]]);
            let points = self.screen(&space.view, self.curve(a, b));
            let path = PaintShape::Path(points);
            draw.stroke(&path, StrokeWidth::Thick, style.halo);
            draw.stroke(&path, StrokeWidth::Medium, style.drawing);
        }
        if let Some(bounds) = ring {
            let painter = ui.content_painter(cx);
            let ppp = painter.pixels_per_point();
            painter.painter.extend(FocusRing::shapes(&style.focus, FocusKind::Halo, &PaintShape::Rect(bounds, egui::CornerRadius::ZERO), ppp));
        }
    }

    /// One node per constraint, named by its number and orientation.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<WideAngleWords>();
        for (index, constraint) in self.session.constraints.iter().enumerate() {
            let orientation = match constraint.orientation {
                ConstraintOrientation::Free => &words.free,
                ConstraintOrientation::Horizontal => &words.horizontal,
                ConstraintOrientation::Vertical => &words.vertical,
            };
            let number = cx.format().integer(index as u64 + 1);
            let name = words.constraint_part.fill(&[("n", number.as_str()), ("orientation", orientation.as_str())]);
            let node = access.node(CONSTRAINT.at(index as u64), Role::LIST_ITEM);
            node.name = craft_ui::contract::Name::Text(name);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> ConstraintLinesOutput {
        ConstraintLinesOutput { response: cx.response.clone(), edit: self.edit, context: self.context }
    }
}
