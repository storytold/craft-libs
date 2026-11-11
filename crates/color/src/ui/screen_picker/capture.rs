//! The capture windows of a screen pick: one over each captured monitor, showing its capture with a crosshair, a
//! loupe and the hint, and reporting the pixel a click picks.

use craft_behavior::{Cursor, CursorIcon, CursorPriority, Metrics, PointerButton, PressPhase, Sense};
use craft_host::target::{WindowChrome, WindowKind};
use craft_theme::TypeRole;
use craft_ui::contract::{
    Common, Component, Container, ContainerProps, Cx, MeasureCx, Mount, NoValue, Outcome, Overlay, Pad, PaintUi, Pointer as _, Props as _, Role,
    Sizing,
};
use craft_ui::primitives::{PaintShape, Revision, StrokeWidth, TextRun, TextureStore, TextureStoreOptions};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::runtime::windows::{WindowLevel, WindowSpec};
use craft_ui::style::{ComponentStyle, DataColor, StyleColor, StyleCx, SurfaceKind};
use craft_window::capture::MonitorArea;
use egui::{ColorImage, Context, CornerRadius, Pos2, Rect, Response, Vec2, pos2, vec2};

use super::{LOUPE_PIXELS, Snapshot};

/// The texture store the captures are shown from; emptied when a pick ends.
const STORE: &str = "craft-color screen pick";

/// The store the capture textures live in.
pub(super) fn store(ctx: &Context) -> TextureStore<u64> {
    TextureStore::in_context(ctx, STORE, TextureStoreOptions::pages())
}

/// The capture's area in the virtual desktop.
pub(super) fn area(snapshot: &Snapshot) -> MonitorArea {
    MonitorArea::new(snapshot.image.origin, snapshot.image.size, snapshot.image.scale)
}

/// The style of a capture window and its view: no padding, and the loupe's inner margin.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(super) struct CaptureStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The loupe's inner margin.
    pad: f32,
    /// How far the loupe sits down and across from the pointer.
    offset: f32,
    /// The centre pixel's outline: the mark over its halo, so it reads on any image.
    mark: StyleColor,
    halo: StyleColor,
}

impl CaptureStyle {
    fn of(cx: &StyleCx<'_>) -> Self {
        let overlay = &cx.tokens.color.overlay;
        CaptureStyle {
            metrics: Metrics::new(),
            pad: cx.tokens.space.gap.inline,
            offset: cx.tokens.size.hit.target,
            mark: cx.color(overlay.stroke),
            halo: cx.color(overlay.halo),
        }
    }
}

/// What a capture window reports: how it closed, if it did.
#[derive(Debug)]
pub(super) struct CaptureWindowOutput {
    response: Response,
    /// Escape or the OS closing the window.
    pub closed: Option<CloseReason>,
}

impl Outcome for CaptureWindowOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A borderless, always-on-top window over one monitor, with no taskbar entry, shown once its first frame is painted.
/// Where OS windows are unavailable it is a modal layer at the same rect.
#[derive(craft_ui::contract::Component)]
#[component(container)]
pub(super) struct CaptureWindow {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    closed: Option<CloseReason>,
}

impl CaptureWindow {
    /// A window titled `title` at `over`, the monitor's rect in its own points.
    pub(super) fn new(title: Label, over: Rect) -> Self {
        let fallback = Overlay::modal().placed(over).body_scroll(false);
        let spec = WindowSpec::dialog()
            .kind(WindowKind::Tool)
            .chrome(WindowChrome::Integrated)
            .captions(None)
            .level(WindowLevel::AlwaysOnTop)
            .taskbar(false)
            .shown_after_first_paint(true);
        let props =
            ContainerProps::new().surface(SurfaceKind::Canvas).pad(Pad::None).sizing(Sizing::Fill).mount(Mount::viewport_with(fallback, spec));
        CaptureWindow { common: Common::new(), props, closed: None }.label(title)
    }
}

impl Component for CaptureWindow {
    const ROLE: Role = Role::GROUP;
    type Style = CaptureStyle;
    type Output = CaptureWindowOutput;

    fn style(cx: &StyleCx<'_>, _: CaptureWindowStyleKey) -> CaptureStyle {
        CaptureStyle::of(cx)
    }

    fn finish(self, cx: Cx<'_, Self>) -> CaptureWindowOutput {
        CaptureWindowOutput { response: cx.response.clone(), closed: self.closed }
    }
}

impl Container for CaptureWindow {
    fn closed(&mut self, why: CloseReason) {
        self.closed = Some(why);
    }
}

/// What a capture view reports in one pass.
#[derive(Debug)]
pub(super) struct CaptureViewOutput {
    response: Response,
    /// A primary click picked this desktop pixel (physical pixels).
    pub picked: Option<Pos2>,
    /// A secondary click asked to cancel.
    pub cancelled: bool,
    /// Wheel notches up (positive) or down this pass.
    pub notches: i32,
}

impl Outcome for CaptureViewOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// One monitor's capture stretched over the view, with a crosshair pointer and the loupe beside it.
#[derive(craft_ui::contract::Component)]
#[component(content_painter)]
pub(super) struct CaptureView<'a> {
    #[common]
    common: Common,
    snapshot: &'a Snapshot,
    /// The capture's key in the texture store.
    key: u64,
    /// Screen pixels per captured pixel in the loupe.
    zoom: usize,
    hints: [Label; 2],
    picked: Option<Pos2>,
    cancelled: bool,
    notches: i32,
}

impl<'a> CaptureView<'a> {
    /// The view of `snapshot`, stored under `key`, its loupe at `zoom`, with the click and wheel `hints`.
    pub(super) fn new(label: impl Into<Label>, snapshot: &'a Snapshot, key: u64, zoom: usize, hints: [Label; 2]) -> Self {
        CaptureView { common: Common::new(), snapshot, key, zoom, hints, picked: None, cancelled: false, notches: 0 }.label(label)
    }

    /// The captured pixel (x, y) shown at `at`, a point of the view at `rect`.
    fn pixel(&self, rect: Rect, at: Pos2) -> Option<(usize, usize)> {
        let area = area(self.snapshot);
        let local = at - rect.min;
        let [x, y] = area.pixel_at([rect.width(), rect.height()], [local.x, local.y])?;
        Some(((x - area.origin[0]) as usize, (y - area.origin[1]) as usize))
    }
}

impl Component for CaptureView<'_> {
    const ROLE: Role = Role::IMAGE;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    const SHOWS_LABEL: bool = false;
    type Style = CaptureStyle;
    type Output = CaptureViewOutput;

    fn style(cx: &StyleCx<'_>, _: CaptureViewStyleKey) -> CaptureStyle {
        CaptureStyle::of(cx)
    }

    /// The capture's size in points: the monitor's own, as it is shown over it.
    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        let [_, _, w, h] = area(self.snapshot).cover();
        vec2(w, h)
    }

    fn sense(&self) -> Sense {
        Sense::HOVER | Sense::CLICK | Sense::CONTEXT_CLICK | Sense::WHEEL_Y
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        cx.cursor(Cursor::System(CursorIcon::Crosshair), CursorPriority::Hover);
        if let Some(wheel) = cx.take_wheel() {
            self.notches = if wheel.scroll.y > 0.0 {
                1
            } else if wheel.scroll.y < 0.0 {
                -1
            } else {
                0
            };
        }
        if let PressPhase::Release { start, pos, click: true, .. } = cx.press().phase {
            match start.button {
                PointerButton::Primary => {
                    let area = area(self.snapshot);
                    self.picked = self.pixel(cx.rect, pos).map(|(x, y)| pos2((area.origin[0] + x as i32) as f32, (area.origin[1] + y as i32) as f32));
                }
                PointerButton::Secondary => self.cancelled = true,
                _ => {}
            }
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let image = &self.snapshot.image;
        let ctx = &cx.response.ctx;
        let texture = store(ctx)
            .get_or_put(ctx, self.key, Revision::default(), || ColorImage::from_rgba_unmultiplied(image.size.map(|n| n as usize), &image.rgba));
        let whole = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
        ui.draw().texture(cx.rect, texture.id, whole);
        let Some(pointer) = cx.response.hover_pos().filter(|at| cx.rect.contains(*at)) else { return };
        let Some((x, y)) = self.pixel(cx.rect, pointer) else { return };
        self.loupe(ui, cx, pointer, (x, y));
    }

    fn finish(self, cx: Cx<'_, Self>) -> CaptureViewOutput {
        CaptureViewOutput { response: cx.response.clone(), picked: self.picked, cancelled: self.cancelled, notches: self.notches }
    }
}

impl CaptureView<'_> {
    /// The loupe beside `pointer`, kept inside the view: the captured pixels round `pixel`, `zoom` screen pixels to
    /// each, the centre one outlined black outside white, then the hex and zoom and the two hints.
    fn loupe(&self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, pointer: Pos2, (x, y): (usize, usize)) {
        let draw = ui.draw();
        let cell = self.zoom as f32 / cx.response.ctx.pixels_per_point();
        let pad = cx.style.pad;
        let grid = LOUPE_PIXELS as f32 * cell;
        let hex = self.snapshot.pixel(x, y).map(|c| format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())).unwrap_or_default();
        let readout = TextRun::new(format!("{hex}   {}\u{d7}", self.zoom)).role(TypeRole::Mono);
        let hints = self.hints.each_ref().map(|hint| TextRun::new(cx.env.resolve(hint).as_str().to_owned()).role(TypeRole::Caption));
        let layouts: Vec<_> = std::iter::once(&readout).chain(hints.iter()).map(|run| draw.layout(run, f32::INFINITY)).collect();
        let text_width = layouts.iter().map(|layout| layout.size().x).fold(0.0, f32::max);
        let text_height: f32 = layouts.iter().map(|layout| layout.size().y).sum();
        let size = vec2(grid.max(text_width) + 2.0 * pad, pad + grid + pad + text_height + pad);
        let wanted = pointer + Vec2::splat(cx.style.offset);
        let at = pos2(wanted.x.min(cx.rect.right() - size.x).max(cx.rect.left()), wanted.y.min(cx.rect.bottom() - size.y).max(cx.rect.top()));
        let area = Rect::from_min_size(at, size);
        draw.surface(area, SurfaceKind::Popup, Default::default());
        let origin = at + vec2((size.x - grid) / 2.0, pad);
        let half = LOUPE_PIXELS / 2;
        for row in 0..LOUPE_PIXELS {
            for col in 0..LOUPE_PIXELS {
                let pixel = (x + col).checked_sub(half).zip((y + row).checked_sub(half)).and_then(|(px, py)| self.snapshot.pixel(px, py));
                let Some(color) = pixel else { continue };
                let rect = Rect::from_min_size(origin + vec2(col as f32, row as f32) * cell, Vec2::splat(cell));
                draw.data_fill(&PaintShape::Rect(rect, CornerRadius::ZERO), DataColor::new(color));
            }
        }
        let centre = Rect::from_min_size(origin + Vec2::splat(half as f32 * cell), Vec2::splat(cell));
        draw.stroke(&PaintShape::Rect(centre.expand(1.0), CornerRadius::ZERO), StrokeWidth::Hairline, cx.style.halo);
        draw.stroke(&PaintShape::Rect(centre, CornerRadius::ZERO), StrokeWidth::Hairline, cx.style.mark);
        let mut line = at + vec2(pad, pad + grid + pad);
        for layout in &layouts {
            draw.galley(line, layout);
            line.y += layout.size().y;
        }
    }
}
