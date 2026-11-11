//! `PreviewPane` and `PreviewZoom`: a picture in a well that the pointer pans and the zoom buttons scale, for the
//! preview of a filter dialog and of an export dialog.

use crate::buttons::IconButton;
use crate::contract::{
    Common, Component, Cx, Focus as _, Input, MeasureCx, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props as _, Role, Show as _,
    ShowContainer as _,
};
use crate::display::{Progress, Text};
use crate::layout_components::Row;
use crate::primitives::ContentPainterCx;
use crate::runtime::access::Access;
use crate::runtime::env::{Env, Label};
use crate::runtime::focus::{ActivateKeys, KeepKeys, KeyUse};
use crate::style::{ComponentStyle, FocusLook, StyleCx};
use craft_behavior::{AccessAction, AccessData, CanvasTransform, CursorIcon, CursorPriority, IconId, Metrics, NavMove, PartKey, PressPhase, Sense};
use egui::load::SizedTexture;
use egui::{Rect, Response, Ui, Vec2, pos2};
use std::ops::RangeInclusive;

#[cfg(feature = "testing")]
mod stories;

/// The well: a drag on it pans the picture and a click centres the view on the point clicked.
const WELL: PartKey = PartKey::new("well");

/// The progress line of a pending pane and the message of a failed one.
const PROGRESS: &str = "progress";
const MESSAGE: &str = "message";

/// The zoom levels of the preview, as fractions of the picture's size.
const ZOOM_STEPS: [f32; 11] = [0.125, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0];

/// The zoom a view allows until its `zoom_range` says otherwise: the first and last of `ZOOM_STEPS`.
const ZOOM_RANGE: RangeInclusive<f32> = RangeInclusive::new(ZOOM_STEPS[0], ZOOM_STEPS[ZOOM_STEPS.len() - 1]);

/// Within this distance two zoom levels are the same level.
const ZOOM_EPSILON: f32 = 1e-4;

/// A picture that overflows the well by no more than this many points on an axis fits it: the fit zoom of a picture
/// that touches both edges can round to a hair over.
const PAN_TOLERANCE: f32 = 0.01;

/// The share of the well an arrow key pans the view by.
const PAN_SHARE: f32 = 0.1;

craft_ui::words! {
    /// The words of the preview pane and its zoom row.
    pub struct PreviewWords {
        zoom_in: "preview.zoomIn" = "Zoom in";
        zoom_out: "preview.zoomOut" = "Zoom out";
        rendering: "preview.rendering" = "Rendering preview";
    }
}

/// The style of a preview pane.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PreviewPaneStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
}

/// What a preview pane shows in its well.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum PreviewState {
    /// The host is rendering: the well with a progress line, `Some` of the fraction done or `None` while unknown.
    Pending(Option<f32>),
    /// The rendered picture.
    Ready(SizedTexture),
    /// The render failed: the well with this message. The label is resolved when the pane is drawn.
    Failed(Label),
}

/// The view of a preview pane, the one value a caller keeps per pane and passes to the pane and to its
/// [`PreviewZoom`]. Both report the next one, so the transform, the fit flag and the zoom range cannot disagree.
///
/// `transform` is a `CanvasTransform` whose canvas origin is the centre of the picture and whose layer origin is the
/// centre of the well: `scale` is the zoom (1 is the picture's own size) and `offset` is how far the picture's centre
/// lies from the well's centre.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PreviewView {
    /// The pan and zoom. While `fit` is set the pane shows the fit view instead, and a pass that reports a fit view
    /// carries it here.
    pub transform: CanvasTransform,
    /// Whether the view follows the well: the zoom is the one that shows the whole picture in it, recomputed when the
    /// well changes. A pan or zoom leaves fit and a double click on the pane returns to it.
    pub fit: bool,
    /// The zoom the wheel, the zoom buttons and fit stay inside.
    pub range: RangeInclusive<f32>,
    /// A fitted view the user cannot change (a survey tile): the pane sizes from the room it is given with no minimum,
    /// takes no wheel zoom and ignores a double click, and the zoom steps leave the view as it is.
    pub fixed: bool,
}

impl PreviewView {
    /// A view at `transform`, not following the well, with the default zoom range (0.125 to 8).
    pub fn new(transform: CanvasTransform) -> Self {
        PreviewView { transform, fit: false, range: ZOOM_RANGE, fixed: false }
    }

    /// A view that follows the well.
    pub fn fitting() -> Self {
        PreviewView { fit: true, ..PreviewView::new(CanvasTransform::new(1.0, Vec2::ZERO)) }
    }

    /// A view that follows the well and cannot be changed: the whole picture in the room given, no zoom, no pan.
    pub fn fixed() -> Self {
        PreviewView { fixed: true, ..PreviewView::fitting() }
    }

    /// The same view with the zoom kept inside `range`.
    #[must_use]
    pub fn zoom_range(mut self, range: RangeInclusive<f32>) -> Self {
        self.range = range;
        self
    }

    /// Whether the zoom can go further in.
    pub fn can_zoom_in(&self) -> bool {
        !self.fixed && PreviewPane::can_zoom_in_within(self.transform, &self.range)
    }

    /// Whether the zoom can go further out.
    pub fn can_zoom_out(&self) -> bool {
        !self.fixed && PreviewPane::can_zoom_out_within(self.transform, &self.range)
    }

    /// The view one zoom level in, with the picture's centre kept in place; it no longer follows the well.
    #[must_use]
    pub fn zoomed_in(&self) -> Self {
        if self.fixed {
            return self.clone();
        }
        PreviewView { transform: PreviewPane::zoom_in_within(self.transform, &self.range), fit: false, ..self.clone() }
    }

    /// The view one zoom level out, with the picture's centre kept in place; it no longer follows the well.
    #[must_use]
    pub fn zoomed_out(&self) -> Self {
        if self.fixed {
            return self.clone();
        }
        PreviewView { transform: PreviewPane::zoom_out_within(self.transform, &self.range), fit: false, ..self.clone() }
    }
}

impl Default for PreviewView {
    fn default() -> Self {
        PreviewView::new(CanvasTransform::new(1.0, Vec2::ZERO))
    }
}

type PaneOverlayFn<'a> = Box<dyn FnOnce(&ContentPainterCx, CanvasTransform, Rect) + 'a>;

/// The host's drawing over a pane's picture: it gets the content painter, the transform the pane drew the picture with
/// (the picture's centre is the canvas origin) and the picture's rect in the painter's coordinates. It runs after the
/// picture and before the pending line or the failed message.
pub struct PaneOverlay<'a>(PaneOverlayFn<'a>);

impl<'a> PaneOverlay<'a> {
    /// The overlay that runs `paint`.
    pub fn new(paint: impl FnOnce(&ContentPainterCx, CanvasTransform, Rect) + 'a) -> Self {
        PaneOverlay(Box::new(paint))
    }

    /// The drawing to run, for a caller that passes the overlay on.
    pub fn into_paint(self) -> PaneOverlayFn<'a> {
        self.0
    }
}

/// What a preview pane reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PreviewOutput {
    /// The pane's response.
    pub response: Response,
    /// The view after a pan, a zoom or a double click of this pass; `None` when the view did not change. A pane that
    /// is not `Ready` reports none.
    pub view: Option<PreviewView>,
    /// The view the pane drew this pass: the given view with its transform settled in the well, or the fit view
    /// while fitting. A caller that keeps the view adopts `view.unwrap_or(shown)`, so a zoom row steps from what is on
    /// screen.
    pub shown: PreviewView,
    /// The well took the second press of a double click this pass. The pane returns to fit on it; a caller may
    /// also act on it (open the picture larger, say).
    pub double_clicked: bool,
}

impl Outcome for PreviewOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A picture in a well, over a checkerboard where it is transparent. A picture smaller than the well sits in the
/// middle and one larger than it never leaves a gap at an edge.
///
/// - Drag pans, a click centres the view on the point clicked (unless `click_centres(false)`), the arrow keys pan, the
///   wheel pans where the picture overflows the well and Ctrl or Cmd with the wheel zooms about the pointer, so the
///   point under it stays. On an axis where the picture fits the well the wheel does not scroll the pane, so the
///   scroll region around it takes it; zoom is always the pane's.
/// - While the [`PreviewView`] is `fit` the zoom is the one that shows the whole picture in the well, recomputed when
///   the well changes. A pan or zoom leaves fit and a double click returns to it.
/// - The pane keeps no view: it shows the one given and reports the next in `PreviewOutput::view`. `PreviewZoom`
///   steps the same view with buttons. Panes that show the same picture area (a 2-up or 4-up dialog) take one view
///   and the dialog applies the first view any of them reports to all.
/// - A pane whose state is not `Ready` draws no picture and reports no view.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(content_painter)]
pub struct PreviewPane<'a> {
    #[common]
    common: Common,
    state: PreviewState,
    view: PreviewView,
    /// Whether a click centres the view on the point clicked.
    #[prop]
    click_centres: bool,
    /// The view after this pass, when the user changed it.
    moved: Option<PreviewView>,
    /// The well took the second press of a double click this pass.
    double_clicked: bool,
    /// The host's drawing over the picture of a `Ready` pane.
    #[prop(func = "FnOnce(&ContentPainterCx, CanvasTransform, Rect)")]
    overlay: Option<PaneOverlay<'a>>,
    /// Whether the picture overflows the well horizontally, and vertically. Set by `parts`, which runs before the
    /// root's sense is read: the wheel scrolls the pane only on an axis it pans on.
    overflow: (bool, bool),
    /// The view is fitted and its zoom range is one value: the pane cannot zoom or pan, so it sizes from the room it
    /// is given and declares no wheel sense.
    #[style_key]
    fixed: bool,
}

impl PreviewPane<'_> {
    /// A pane named `label` showing `texture` through `view`.
    pub fn new(label: impl Into<Label>, texture: SizedTexture, view: &PreviewView) -> Self {
        Self::state(label, PreviewState::Ready(texture), view)
    }

    /// A pane named `label` showing `state` through `view`.
    pub fn state(label: impl Into<Label>, state: PreviewState, view: &PreviewView) -> Self {
        PreviewPane {
            common: Common::new(),
            state,
            view: view.clone(),
            click_centres: true,
            moved: None,
            double_clicked: false,
            overlay: None,
            overflow: (false, false),
            fixed: view.fixed,
        }
        .label(label)
    }

    /// The view at the next zoom level above the view's, with the picture's centre kept in place.
    pub fn zoom_in(view: CanvasTransform) -> CanvasTransform {
        Self::zoom_in_within(view, &ZOOM_RANGE)
    }

    /// The view at the next zoom level below the view's, with the picture's centre kept in place.
    pub fn zoom_out(view: CanvasTransform) -> CanvasTransform {
        Self::zoom_out_within(view, &ZOOM_RANGE)
    }

    /// Whether `view` can zoom further in.
    pub fn can_zoom_in(view: CanvasTransform) -> bool {
        Self::can_zoom_in_within(view, &ZOOM_RANGE)
    }

    /// Whether `view` can zoom further out.
    pub fn can_zoom_out(view: CanvasTransform) -> bool {
        Self::can_zoom_out_within(view, &ZOOM_RANGE)
    }

    /// `zoom_in` with the zoom kept inside `range`: the levels are the base ones inside it and its two ends.
    pub fn zoom_in_within(view: CanvasTransform, range: &RangeInclusive<f32>) -> CanvasTransform {
        let next = Self::levels(range).into_iter().find(|level| *level > view.scale + ZOOM_EPSILON).unwrap_or(*range.end());
        Self::rescaled(view, next, range, Vec2::ZERO)
    }

    /// `zoom_out` with the zoom kept inside `range`.
    pub fn zoom_out_within(view: CanvasTransform, range: &RangeInclusive<f32>) -> CanvasTransform {
        let next = Self::levels(range).into_iter().rev().find(|level| *level < view.scale - ZOOM_EPSILON).unwrap_or(*range.start());
        Self::rescaled(view, next, range, Vec2::ZERO)
    }

    /// Whether `view` can zoom further in inside `range`.
    pub fn can_zoom_in_within(view: CanvasTransform, range: &RangeInclusive<f32>) -> bool {
        view.scale < *range.end() - ZOOM_EPSILON
    }

    /// Whether `view` can zoom further out inside `range`.
    pub fn can_zoom_out_within(view: CanvasTransform, range: &RangeInclusive<f32>) -> bool {
        view.scale > *range.start() + ZOOM_EPSILON
    }

    /// The view that shows the whole picture of `picture` points in a well of `well` points, with its zoom kept
    /// inside `range`.
    pub fn fit_view(picture: Vec2, well: Vec2, range: &RangeInclusive<f32>) -> CanvasTransform {
        let scale = if picture.x > 0.0 && picture.y > 0.0 { (well.x / picture.x).min(well.y / picture.y) } else { 1.0 };
        CanvasTransform::new(scale.clamp(*range.start(), *range.end()), Vec2::ZERO)
    }

    /// The zoom levels inside `range`, ascending: its ends and the base levels between them.
    fn levels(range: &RangeInclusive<f32>) -> Vec<f32> {
        let inside = ZOOM_STEPS.iter().copied().filter(|step| *step > *range.start() + ZOOM_EPSILON && *step < *range.end() - ZOOM_EPSILON);
        std::iter::once(*range.start()).chain(inside).chain(std::iter::once(*range.end())).collect()
    }

    /// `view` at zoom `scale` (kept inside `range`), the point `about` (from the well's centre) staying where it is.
    fn rescaled(view: CanvasTransform, scale: f32, range: &RangeInclusive<f32>, about: Vec2) -> CanvasTransform {
        let scale = scale.clamp(*range.start(), *range.end());
        let ratio = scale / view.scale;
        CanvasTransform::new(scale, about - (about - view.offset) * ratio)
    }

    /// `view` with its offset moved to where the picture of `picture` points shown in a well of `well` points allows:
    /// centred on an axis where the picture fits, else not further than the picture's edge at the well's edge.
    pub fn settled(view: CanvasTransform, picture: Vec2, well: Vec2) -> CanvasTransform {
        let slack = ((picture * view.scale - well) / 2.0).max(Vec2::ZERO);
        CanvasTransform::new(view.scale, view.offset.clamp(-slack, slack))
    }

    /// The picture of a `Ready` pane.
    fn picture(&self) -> Option<SizedTexture> {
        match &self.state {
            PreviewState::Ready(texture) => Some(*texture),
            _ => None,
        }
    }

    /// How far the picture's centre may lie from the well's centre on each axis: 0 where the picture fits.
    fn slack(&self, well: Vec2) -> Vec2 {
        match self.picture() {
            Some(texture) => ((texture.size * self.shown(well).scale - well) / 2.0).max(Vec2::ZERO),
            None => Vec2::ZERO,
        }
    }

    /// Whether the picture overflows a well of `well` points horizontally and vertically, so the view can pan there.
    fn pans(&self, well: Vec2) -> (bool, bool) {
        let slack = self.slack(well);
        (slack.x > PAN_TOLERANCE, slack.y > PAN_TOLERANCE)
    }

    /// The transform the pane shows in a well of `well` points: the given one settled in the well, the fit view while
    /// fitting, and the given one as it is where there is no picture.
    fn shown(&self, well: Vec2) -> CanvasTransform {
        let Some(texture) = self.picture() else { return self.view.transform };
        let view = if self.view.fit { Self::fit_view(texture.size, well, &self.view.range) } else { self.view.transform };
        Self::settled(view, texture.size, well)
    }
}

impl Component for PreviewPane<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_keep(KeepKeys::BlockArrows).with_activate(ActivateKeys::Neither);
    type Style = PreviewPaneStyle;
    type Output = PreviewOutput;

    fn style(cx: &StyleCx<'_>, key: PreviewPaneStyleKey) -> PreviewPaneStyle {
        PreviewPaneStyle {
            metrics: if key.fixed {
                Metrics::new()
            } else {
                Metrics::new().min_size(Vec2::new(cx.tokens.size.plot.width, cx.tokens.size.plot.height))
            },
            focus: cx.focus_look(),
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        let (x, y) = self.overflow;
        let mut sense = Sense::FOCUS;
        if !self.fixed {
            sense |= Sense::WHEEL_ZOOM;
        }
        if x {
            sense |= Sense::WHEEL_X;
        }
        if y {
            sense |= Sense::WHEEL_Y;
        }
        sense
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let (x, y) = self.pans(cx.rect.size());
        access.pans(y, x);
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        self.overflow = self.pans(rect.size());
        let part = cx.part(WELL, rect).sense(Sense::CLICK | Sense::DRAG).pointer_only("the arrow keys pan the view");
        if self.overflow != (false, false) {
            part.cursor(CursorIcon::Grab);
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(texture) = self.picture() else { return };
        let well = cx.rect;
        let shown = self.shown(well.size());
        let mut next = shown;
        // The second press of a double click returns to fit, as the other double-click readers detect it. The pane
        // reports a view only when the result differs from what it shows, so the press staying down changes nothing.
        let mut refit = false;
        match cx.part(WELL).press.phase {
            PressPhase::Drag { delta, .. } => {
                next.offset += delta;
                cx.cursor(CursorIcon::Grabbing, CursorPriority::Drag);
            }
            PressPhase::Down { start, .. } if start.clicks >= 2 && !self.fixed => {
                refit = true;
                self.double_clicked = true;
            }
            PressPhase::Release { start, pos, click: true, .. } if self.click_centres && start.clicks < 2 => {
                next.offset += well.center() - pos;
            }
            _ => {}
        }
        let well_size = well.size();
        let pan = |mv: NavMove| match mv {
            NavMove::Forward => Some(-Vec2::X * well_size * PAN_SHARE),
            NavMove::Back => Some(Vec2::X * well_size * PAN_SHARE),
            NavMove::Down => Some(-Vec2::Y * well_size * PAN_SHARE),
            NavMove::Up => Some(Vec2::Y * well_size * PAN_SHARE),
            _ => None,
        };
        while let Some((mv, _)) = cx.intents().take_move() {
            next.offset += pan(mv).unwrap_or(Vec2::ZERO);
        }
        // AccessKit's scroll actions are the arrows for assistive technology: the same pans.
        let rtl = cx.flow.is_rtl();
        while let Some(request) = cx.intents().take_scroll_access(false) {
            match (request.action, request.data) {
                (AccessAction::ScrollRight, _) => next.offset += pan(if rtl { NavMove::Back } else { NavMove::Forward }).unwrap_or(Vec2::ZERO),
                (AccessAction::ScrollLeft, _) => next.offset += pan(if rtl { NavMove::Forward } else { NavMove::Back }).unwrap_or(Vec2::ZERO),
                (AccessAction::ScrollDown, _) => next.offset += pan(NavMove::Down).unwrap_or(Vec2::ZERO),
                (AccessAction::ScrollUp, _) => next.offset += pan(NavMove::Up).unwrap_or(Vec2::ZERO),
                (AccessAction::SetScrollOffset, Some(AccessData::SetScrollOffset(point))) => {
                    // A scroll offset counts from the picture's top-left edge in the well; the view offset from its centre.
                    let slack = self.slack(well_size);
                    next.offset = Vec2::new(slack.x - point.x as f32, slack.y - point.y as f32);
                }
                _ => {}
            }
        }
        if let Some(wheel) = cx.take_wheel() {
            if (wheel.zoom - 1.0).abs() > ZOOM_EPSILON {
                let about = cx.hover_pos(WELL).map_or(Vec2::ZERO, |at| at - well.center());
                next = Self::rescaled(next, next.scale * wheel.zoom, &self.view.range, about);
            } else {
                next.offset += wheel.scroll;
            }
        }
        let drawn = PreviewView { transform: shown, ..self.view.clone() };
        let after = if refit {
            PreviewView {
                transform: Self::settled(Self::fit_view(texture.size, well_size, &self.view.range), texture.size, well_size),
                fit: true,
                ..drawn.clone()
            }
        } else {
            let next = Self::settled(next, texture.size, well_size);
            if next == shown { drawn.clone() } else { PreviewView { transform: next, fit: false, ..drawn.clone() } }
        };
        self.moved = (after != drawn).then_some(after);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let inside = {
            let painter = ui.content_painter(cx);
            let inside = painter.plot_frame(cx.rect);
            match &self.state {
                PreviewState::Ready(texture) => {
                    let view = self.shown(cx.rect.size());
                    let size = texture.size * view.scale;
                    let picture = Rect::from_center_size(cx.rect.center() + view.offset, size);
                    let visible = picture.intersect(inside);
                    if size.x > 0.0 && size.y > 0.0 && visible.width() > 0.0 && visible.height() > 0.0 {
                        painter.checker(visible);
                        let uv = Rect::from_min_max(
                            pos2((visible.min.x - picture.min.x) / size.x, (visible.min.y - picture.min.y) / size.y),
                            pos2((visible.max.x - picture.min.x) / size.x, (visible.max.y - picture.min.y) / size.y),
                        );
                        painter.picture(texture.id, visible, uv, 0.0);
                    }
                    if let Some(PaneOverlay(overlay)) = self.overlay.take() {
                        overlay(&painter, view, picture);
                    }
                }
                PreviewState::Pending(_) => painter.checker(inside),
                PreviewState::Failed(_) => {}
            }
            inside
        };
        let (track, thickness, gap) =
            ui.with_style(cx.resolved.variant, |style| (style.tokens.size.length.track, style.track().thickness, style.tokens.space.gap.inline));
        let rendering = ui.env.words::<PreviewWords>().rendering.clone();
        match &self.state {
            PreviewState::Pending(done) => {
                let bar = Rect::from_center_size(inside.center(), Vec2::new(track.min(inside.width()), thickness));
                match done {
                    Some(fraction) => ui.child(PROGRESS, bar, Progress::new(*fraction).label(rendering)),
                    None => ui.child(PROGRESS, bar, Progress::indeterminate().label(rendering)),
                };
            }
            PreviewState::Failed(message) => {
                ui.child(MESSAGE, inside.shrink(gap), Text::new(message.clone()));
            }
            PreviewState::Ready(_) => {}
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> PreviewOutput {
        let shown = PreviewView { transform: self.shown(cx.rect.size()), ..self.view.clone() };
        PreviewOutput { response: cx.response.clone(), view: self.moved, shown, double_clicked: self.double_clicked }
    }
}

/// What a zoom row reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PreviewZoomOutput {
    /// The row's response.
    pub response: Response,
    /// The view after a button of this pass, which no longer follows the well; `None` when none was pressed.
    pub view: Option<PreviewView>,
}

impl Outcome for PreviewZoomOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The zoom row of a preview: a zoom-out button, the zoom as a percentage and a zoom-in button.
#[must_use = "show the zoom row with `show`"]
#[derive(crate::contract::Pattern)]
pub struct PreviewZoom {
    view: PreviewView,
}

impl PreviewZoom {
    /// A zoom row for `view`, the one the pane takes: the buttons stay inside its zoom range. While the pane fits,
    /// the caller has adopted the pane's `PreviewOutput::shown`, so the row steps from what is on screen.
    pub fn new(view: &PreviewView) -> Self {
        PreviewZoom { view: view.clone() }
    }

    /// Shows the row.
    pub fn show(self, ui: &mut Ui) -> PreviewZoomOutput {
        let env = Env::of(ui);
        let words = env.words::<PreviewWords>();
        let readout = env.units().percent(f64::from(self.view.transform.scale) * 100.0);
        let mut next = None;
        let row = Row::new().show(ui, |ui, _| {
            let out = IconButton::new(IconId::new("minus")).label(words.zoom_out.clone()).enabled(self.view.can_zoom_out()).show(ui);
            Text::new(readout.clone()).show(ui);
            let into = IconButton::new(IconId::new("plus")).label(words.zoom_in.clone()).enabled(self.view.can_zoom_in()).show(ui);
            if crate::contract::clicked(&out) {
                next = Some(self.view.zoomed_out());
            }
            if crate::contract::clicked(&into) {
                next = Some(self.view.zoomed_in());
            }
        });
        PreviewZoomOutput { response: row.output, view: next }
    }
}
