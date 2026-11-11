//! `Navigator`: the preview box with the proxy rectangle of the view. The picture is fitted inside the box; a press
//! moves the view, a drag of the proxy moves it by the distance dragged, a Cmd/Ctrl-drag zooms to the rectangle, the
//! wheel and `+` and `-` zoom about the view centre, and the arrow and page keys move it.

use super::proxy::{preview_rect, proxy_quad, view_center_for, zoom_to_document_rect};
use crate::canvas::zoom_motion::{self, Steer};
use crate::canvas::{CanvasWords, NavigationOptions, PageKey, ViewCommands, WheelAction, WheelNavigation};
use crate::canvas_data::ProxyColor;
use crate::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, Input, MeasureCx, Name, Outcome, PaintUi, PartsCx,
    Pointer as _, Props as _, Remember as _, Role, WordSetCx as _,
};
use crate::primitives::textures::CachedTexture;
use crate::primitives::{ContentPainterCx, PaintShape};
use crate::runtime::access::Numeric;
use crate::runtime::commands;
use crate::runtime::env::Label;
use crate::runtime::focus::{ActivateKeys, KeepKeys, KeyUse};
use crate::style::{ComponentStyle, FocusLook, StyleCx, UiTokens};
use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{
    AccessAction, AccessData, CancelReason, CanvasView, CursorIcon, CursorPriority, HitLevel, HitPriority, HitShape, Metrics, NavMove, PartKey,
    PressCancel, PressPhase, Sense,
};
use craft_keymap::{Chord, Key, Mods};
use egui::{Color32, CornerRadius, Pos2, Rect, Response, Vec2, pos2};
use std::ops::Deref;

pub(super) const WELL: PartKey = PartKey::new("well");
const PROXY: PartKey = PartKey::new("proxy");
/// The accessible scroll bars: sub-nodes with no part of their own.
const HORIZONTAL: PartKey = PartKey::new("horizontal");
const VERTICAL: PartKey = PartKey::new("vertical");
/// The document's picture: an image node with no part of its own.
const PICTURE: PartKey = PartKey::new("picture");

/// The keyboard path of the well and the proxy.
const POINTER_ONLY: &str = "the arrow and page keys move the view and + and - step the zoom; the zoom rectangle has no numeric form";

/// An arrow key, an AccessKit scroll or a scroll bar's Increment moves the view by this share of the visible span,
/// Shift with an arrow by `SHIFT_STEP`.
const ARROW_STEP: f32 = 0.1;
const SHIFT_STEP: f32 = 0.5;

/// The keys that step the zoom ladder, with the direction of the step.
const ZOOM_KEYS: [(Chord, i32); 4] =
    [(Chord::plain(Key::Plus), 1), (Chord::plain(Key::Equals), 1), (Chord::new(Mods::SHIFT, Key::Equals), 1), (Chord::plain(Key::Minus), -1)];

type NavigatorPaintFn<'a> = Box<dyn FnMut(&ContentPainterCx<'_>, Rect) + 'a>;

/// A closure that paints the document's picture into the fitted rect.
pub struct NavigatorPaint<'a>(NavigatorPaintFn<'a>);

impl<'a> NavigatorPaint<'a> {
    /// A painter that draws the picture with `paint`, given the content painter and the rect the picture fills.
    pub fn new(paint: impl FnMut(&ContentPainterCx<'_>, Rect) + 'a) -> Self {
        NavigatorPaint(Box::new(paint))
    }
}

/// What the preview box shows: the document's picture, made and cached by the app.
#[non_exhaustive]
pub enum NavigatorPreview<'a> {
    /// A texture from the app's own texture store, drawn over the checkerboard into the fitted rect.
    Texture(CachedTexture),
    /// The app paints the picture itself into the fitted rect; the checkerboard is not drawn.
    Paint(NavigatorPaint<'a>),
    /// No picture yet: the checkerboard alone, with the proxy over it.
    None,
}

impl NavigatorPreview<'_> {
    /// The same picture lent for one run of a body that may run more than once per pass.
    pub fn reborrowed(&mut self) -> NavigatorPreview<'_> {
        match self {
            NavigatorPreview::Texture(texture) => NavigatorPreview::Texture(*texture),
            NavigatorPreview::Paint(NavigatorPaint(paint)) => NavigatorPreview::Paint(NavigatorPaint::new(move |painter, rect| paint(painter, rect))),
            NavigatorPreview::None => NavigatorPreview::None,
        }
    }
}

/// The press in progress, kept between passes.
#[derive(Clone, Default)]
struct Held {
    active: bool,
    /// The press draws a zoom rectangle (Cmd or Ctrl at the press).
    zoom: bool,
    /// The press started on the proxy: the view moves by the distance dragged.
    on_proxy: bool,
    from: Option<CanvasView>,
}

/// The style of the navigator.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct NavigatorStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// Between the box and the fitted document.
    inset: f32,
    /// The corners of an upright proxy.
    radius: CornerRadius,
}

/// The preview box of the Navigator panel with the proxy rectangle of the view.
///
/// - A press on the box moves the view centre to the point pressed and follows the pointer; a press on the proxy moves
///   the view by the distance dragged. Escape puts the view back as it was at the press.
/// - With Cmd (macOS) or Ctrl held at the press, the drag draws a rectangle and the release zooms the view to it.
/// - The wheel zooms about the view centre, one notch at a time, whether or not Zoom with Scroll Wheel is on; a pinch
///   zooms the same way when the navigation options allow it.
/// - Focused, the arrow keys move the view by a tenth of its span (Shift: half); Page Up, Page Down, Home and End
///   scroll as on the canvas ([`PageKey`]); `+` and `-` step the zoom ladder, and a step past a limit changes nothing.
/// - With Animated Zoom on, a zoom (the wheel, `+` and `-`, the zoom rectangle) eases on the canvas whose view the
///   navigator edits: the bound view takes the target, and the canvas that the view commands reach (the one the zoom
///   row's Zoom In steps) shows the way there in its one zoom ease.
/// - While a press is held, the wheel, the keys and AccessKit requests change nothing.
/// - The box pans for AccessKit (the scroll actions move the view as the arrows do) and has two scroll-bar nodes, the
///   view centre in percent of the document, whose Increment and Decrement move the view by the arrow step and whose
///   set value moves the view.
/// - The proxy follows a rotated or mirrored view as a quad.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(content_painter)]
pub struct Navigator<'a> {
    #[common]
    common: Common,
    view: Bind<'a, CanvasView>,
    document: Size,
    /// The picture.
    #[prop]
    preview: NavigatorPreview<'a>,
    /// The app's navigation options.
    #[prop]
    navigation: NavigationOptions,
    /// The colour of the proxy rectangle.
    #[prop]
    proxy: ProxyColor,
    zoom_rect: Option<Rect>,
}

impl<'a> Navigator<'a> {
    /// A navigator named `label` that edits `view`, over a document of `document` units.
    pub fn new(label: impl Into<Label>, view: Bind<'a, CanvasView>, document: Size) -> Self {
        Navigator {
            common: Common::new(),
            view,
            document,
            preview: NavigatorPreview::None,
            navigation: NavigationOptions::default(),
            proxy: ProxyColor::Default,
            zoom_rect: None,
        }
        .label(label)
    }

    fn quad(&self, view: &CanvasView, rect: Rect, inset: f32) -> Vec<Pos2> {
        clip_polygon(&proxy_quad(view, self.document, preview_rect(rect, self.document, inset)), rect)
    }
}

/// The part of the convex polygon `points` inside `rect` (Sutherland and Hodgman).
fn clip_polygon(points: &[Pos2], rect: Rect) -> Vec<Pos2> {
    let edges: [fn(Pos2, Rect) -> f32; 4] = [|p, r| p.x - r.min.x, |p, r| r.max.x - p.x, |p, r| p.y - r.min.y, |p, r| r.max.y - p.y];
    let mut out = points.to_vec();
    for inside in edges {
        let input = std::mem::take(&mut out);
        for (at, &current) in input.iter().enumerate() {
            let previous = input[(at + input.len() - 1) % input.len()];
            let (a, b) = (inside(previous, rect), inside(current, rect));
            if (a >= 0.0) != (b >= 0.0) {
                let t = a / (a - b);
                out.push(previous + (current - previous) * t);
            }
            if b >= 0.0 {
                out.push(current);
            }
        }
        if out.is_empty() {
            break;
        }
    }
    out
}

/// The part of the segment `a`-`b` inside `rect` (Liang and Barsky), if any.
fn clip_segment(a: Pos2, b: Pos2, rect: Rect) -> Option<[Pos2; 2]> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0_f32, 1.0_f32);
    for (p, q) in [(-d.x, a.x - rect.min.x), (d.x, rect.max.x - a.x), (-d.y, a.y - rect.min.y), (d.y, rect.max.y - a.y)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    (t0 <= t1).then(|| [a + d * t0, a + d * t1])
}

/// `view` with its centre at `center` inside the document.
fn centred(view: &CanvasView, center: Point, document: Size) -> CanvasView {
    let mut next = *view;
    next.center = Point::new(center.x.clamp(0.0, document.width), center.y.clamp(0.0, document.height));
    next
}

/// Applies the arrow, page, scroll-request and set-value intents of a pass to `next`, unless a press is `running`.
fn take_navigation(cx: &mut Cx<'_, Navigator<'_>>, next: &mut CanvasView, running: bool, document: Size) {
    let span = next.rect.size();
    let side = if cx.flow.is_rtl() { -1.0 } else { 1.0 };
    let mut delta = Vec2::ZERO;
    while let Some((mv, mods)) = cx.intents().take_move() {
        if running {
            continue;
        }
        let share = if mods.extend { SHIFT_STEP } else { ARROW_STEP };
        match mv {
            NavMove::Back => delta.x -= side * span.x * share,
            NavMove::Forward => delta.x += side * span.x * share,
            NavMove::Up => delta.y -= span.y * share,
            NavMove::Down => delta.y += span.y * share,
            _ => {
                if let Some(key) = PageKey::of(mv, mods, span) {
                    key.apply(next, document);
                }
            }
        }
    }
    // AccessKit's scroll actions are the arrows for assistive technology.
    while let Some(request) = cx.intents().take_scroll_access(false) {
        if running {
            continue;
        }
        match (request.action, request.data) {
            (AccessAction::ScrollRight, _) => delta.x += span.x * ARROW_STEP,
            (AccessAction::ScrollLeft, _) => delta.x -= span.x * ARROW_STEP,
            (AccessAction::ScrollDown, _) => delta.y += span.y * ARROW_STEP,
            (AccessAction::ScrollUp, _) => delta.y -= span.y * ARROW_STEP,
            // The offset is the document point at the visible area's top-left corner.
            (AccessAction::SetScrollOffset, Some(AccessData::SetScrollOffset(offset))) => {
                let visible = next.visible();
                next.center = Point::new(offset.x + visible.width() / 2.0, offset.y + visible.height() / 2.0);
            }
            _ => {}
        }
    }
    if delta != Vec2::ZERO {
        let by = next.unmap_vec(delta);
        next.center = Point::new(next.center.x + by.x, next.center.y + by.y);
    }
    let visible = next.visible();
    let bars = |request: &craft_behavior::AccessRequest| {
        matches!(request.action, AccessAction::Increment | AccessAction::Decrement)
            && [HORIZONTAL, VERTICAL].into_iter().any(|key| request.target == Some(key.into()))
    };
    while let Some(request) = cx.intents().take_access_where(bars) {
        if running {
            continue;
        }
        let by = if request.action == AccessAction::Increment { f64::from(ARROW_STEP) } else { -f64::from(ARROW_STEP) };
        if request.target == Some(HORIZONTAL.into()) {
            next.center.x += by * visible.width();
        } else {
            next.center.y += by * visible.height();
        }
    }
    if let Some(percent) = cx.intents().take_set_value_at(HORIZONTAL).filter(|percent| percent.is_finite() && !running) {
        next.center.x = (percent / 100.0).clamp(0.0, 1.0) * document.width;
    }
    if let Some(percent) = cx.intents().take_set_value_at(VERTICAL).filter(|percent| percent.is_finite() && !running) {
        next.center.y = (percent / 100.0).clamp(0.0, 1.0) * document.height;
    }
}

impl Component for Navigator<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_keep(KeepKeys::BlockArrows).with_activate(ActivateKeys::Neither);
    type Style = NavigatorStyle;
    type Output = NavigatorOutput;

    fn style(cx: &StyleCx<'_>, _: NavigatorStyleKey) -> NavigatorStyle {
        NavigatorStyle {
            metrics: Metrics::new().min_size(Vec2::new(cx.tokens.size.plot.width, cx.tokens.size.plot.height)),
            focus: cx.focus_look(),
            inset: cx.tokens.space.gap.inline,
            radius: UiTokens::corner(cx.tokens.shape.radius.sm),
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::FOCUS | Sense::WHEEL | Sense::WHEEL_ZOOM
    }

    fn bind(&mut self) -> Option<BindMut<'_, CanvasView>> {
        Some(self.view.reborrow())
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.part(WELL, rect).sense(Sense::CLICK | Sense::DRAG).pointer_only(POINTER_ONLY);
        let Some(view) = cx.value().copied() else { return };
        let quad = self.quad(&view, rect, cx.style.inset);
        if quad.len() >= 3 {
            cx.part(PROXY, HitShape::Polygon(quad))
                .sense(Sense::DRAG)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .cursor(CursorIcon::Grab)
                .drag_cursor(CursorIcon::Grabbing)
                .pointer_only(POINTER_ONLY);
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(view) = cx.value().copied() else { return };
        let rect = cx.rect;
        let document = self.document;
        let ppp = cx.env.scale().0;
        let fitted = preview_rect(rect, document, cx.style.inset);
        let scale = f64::from(fitted.width() / document.width.max(1.0) as f32);

        let proxy = cx.part_if_declared(PROXY).map(|part| (part.press, part.hovered));
        let well = (cx.part(WELL).press, cx.part(WELL).hovered);
        let (press, hovered, on_proxy) = match proxy {
            Some((press, hovered)) if press.phase.is_press() => (press, hovered, true),
            Some((_, true)) => (well.0, true, true),
            _ => (well.0, well.1, false),
        };
        if hovered && press.mods.cmd && !press.phase.is_press() {
            cx.cursor(CursorIcon::ZoomIn, CursorPriority::Part);
        }
        let mut held = cx.memory::<Held>().clone();
        if let PressPhase::Down { start, .. } = press.phase {
            held = Held { active: true, zoom: press.mods.cmd, on_proxy, from: Some(view) };
            if !held.zoom {
                cx.begin();
                if !on_proxy {
                    cx.propose(centred(&view, view_center_for(fitted, document, start.origin), document));
                }
            }
        }
        // The view the zoom rectangle asks for at its release.
        let mut boxed = None;
        if held.active {
            let from = held.from.unwrap_or(view);
            let (zooming, moves_by_drag) = (held.zoom, held.on_proxy);
            let at = |pos: Pos2, origin: Pos2| {
                if moves_by_drag {
                    let moved = (pos - origin) / scale as f32;
                    centred(&from, Point::new(from.center.x + f64::from(moved.x), from.center.y + f64::from(moved.y)), document)
                } else {
                    centred(&from, view_center_for(fitted, document, pos), document)
                }
            };
            match press.phase {
                PressPhase::DragStart { start, pos, .. } | PressPhase::Drag { start, pos, .. } if zooming => {
                    self.zoom_rect = Some(Rect::from_two_pos(start.origin, pos).intersect(fitted));
                }
                PressPhase::DragStart { start, pos, .. } | PressPhase::Drag { start, pos, .. } => cx.propose(at(pos, start.origin)),
                PressPhase::Release { start, pos, .. } if zooming => {
                    let r = Rect::from_two_pos(start.origin, pos).intersect(fitted);
                    let to_doc = |p: Pos2| ((p.x - fitted.min.x) / scale as f32, (p.y - fitted.min.y) / scale as f32);
                    let ((x0, y0), (x1, y1)) = (to_doc(r.min), to_doc(r.max));
                    let mut next = view;
                    if zoom_to_document_rect(
                        &mut next,
                        craft_behavior::kurbo::Rect::new(f64::from(x0), f64::from(y0), f64::from(x1), f64::from(y1)),
                        document,
                        ppp,
                    ) {
                        boxed = Some(next);
                    }
                    held.active = false;
                }
                PressPhase::Release { start, pos, .. } => {
                    cx.propose(at(pos, start.origin));
                    cx.commit();
                    held.active = false;
                }
                PressPhase::Cancel { reason, .. } => {
                    if !zooming {
                        cx.cancel(if reason == PressCancel::Escape { CancelReason::Escape } else { CancelReason::Blur });
                    }
                    held.active = false;
                }
                _ => {}
            }
        }
        let running = held.active || cx.gesture().is_some();
        *cx.memory::<Held>() = held;

        // While a press is held, the wheel, the keys and AccessKit requests change nothing; they are still taken.
        let view = cx.value().copied().unwrap_or(view);
        let mut next = boxed.unwrap_or(view);
        let mut zoomed = boxed.is_some();
        if let Some(wheel) = cx.take_wheel()
            && !running
        {
            let options = NavigationOptions { zoom_with_wheel: true, ..self.navigation };
            let action = cx.memory::<WheelNavigation>().read(&wheel, &options);
            if let Some(action @ (WheelAction::Zoom(_) | WheelAction::SnapZoom(_))) = action {
                let before = next;
                WheelNavigation::apply(&mut next, action, before.rect.center(), document, ppp);
                next.center = before.center;
                zoomed |= next.zoom != before.zoom;
            }
        }
        for (chord, dir) in ZOOM_KEYS {
            while cx.take_claimed(chord) {
                if !running {
                    zoomed |= zoom::step(&mut next, dir, document, ppp);
                }
            }
            cx.claim(chord);
        }

        take_navigation(cx, &mut next, running, document);

        let next = centred(&next, next.center, document);
        if next == view {
            return;
        }
        if zoomed
            && self.navigation.animated_zoom
            && let Some(canvas) = commands::resolve(&cx.core.ctx, ViewCommands::ZoomIn).instance
        {
            // A zoom that keeps the centre eases about it; the zoom rectangle moves the centre, so it eases freely.
            let anchor = (next.center == view.center).then_some((view.center, view.rect.center()));
            zoom_motion::steer(&cx.core.ctx, canvas, Steer::Ease { from: view, anchor });
        }
        cx.set(next);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let rect = cx.rect;
        let style = cx.style;
        let fitted = preview_rect(rect, self.document, style.inset);
        let preview = std::mem::replace(&mut self.preview, NavigatorPreview::None);
        let painter = ui.content_painter(cx);
        let inside = painter.plot_frame(rect);
        match preview {
            NavigatorPreview::Texture(texture) => {
                painter.checker(fitted);
                painter.picture(texture.id, fitted, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), 0.0);
            }
            NavigatorPreview::Paint(NavigatorPaint(mut paint)) => paint(&painter, fitted),
            NavigatorPreview::None => painter.checker(fitted),
        }
        let Some(view) = cx.value().copied() else { return };
        // Upright, the proxy is a rounded rectangle; turned or mirrored, the quad's edges, each clipped to the box, so
        // no line runs along the box's edge.
        let quad = proxy_quad(&view, self.document, fitted);
        let shapes: Vec<PaintShape> = if view.axis_aligned() {
            let proxy = Rect::from_points(&quad).intersect(inside);
            proxy.is_positive().then_some(PaintShape::Rect(proxy, style.radius)).into_iter().collect()
        } else {
            (0..4).filter_map(|at| clip_segment(quad[at], quad[(at + 1) % 4], inside)).map(|edge| PaintShape::Path(edge.to_vec())).collect()
        };
        painter.outline_in(&shapes, Color32::from(self.proxy.color()));
        if let Some(zoom) = self.zoom_rect.filter(|zoom| zoom.is_positive()) {
            painter.marching_ants(&[zoom.left_top(), zoom.right_top(), zoom.right_bottom(), zoom.left_bottom()], true, 0.0);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let Some(view) = cx.value().copied() else { return };
        access.pans(true, true);
        let words = cx.words::<CanvasWords>();
        access.node(PICTURE, Role::IMAGE).name = Name::Text(words.navigator_preview.as_str().to_owned());
        let visible = view.visible();
        let percent = |length: f64, extent: f64| length / extent.max(1.0) * 100.0;
        for (key, name, center, extent, span) in [
            (HORIZONTAL, &words.navigator_horizontal, view.center.x, self.document.width, visible.width()),
            (VERTICAL, &words.navigator_vertical, view.center.y, self.document.height, visible.height()),
        ] {
            let node = access.node(key, Role::SCROLL_BAR);
            node.name = Name::Text(name.as_str().to_owned());
            let numeric = Numeric::new(percent(center, extent).clamp(0.0, 100.0), 0.0..=100.0);
            node.numeric = Some(numeric.step(percent(span * f64::from(ARROW_STEP), extent)));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> NavigatorOutput {
        NavigatorOutput { view: cx.edit_response() }
    }
}

/// What the navigator reports in a pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct NavigatorOutput {
    /// The view: a drag's gesture, a key, an AccessKit request or a zoom, as one change. A zoom with Animated Zoom on
    /// is the change to its target; the canvas eases to it.
    pub view: EditResponse<CanvasView>,
}

impl Deref for NavigatorOutput {
    type Target = EditResponse<CanvasView>;

    fn deref(&self) -> &Self::Target {
        &self.view
    }
}

impl Outcome for NavigatorOutput {
    type Value = CanvasView;

    fn response(&self) -> &Response {
        &self.view.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.view.story_facts()
    }
}
