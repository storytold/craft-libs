//! Floating window chrome: an opaque frame with anchored resize handles, and a title bar with an
//! optional title, ▾ menu button and ×. [`super::DockArea`] draws its floating windows with
//! these; other hosts (task panes) use them with their own ids and actions. Nothing here changes
//! a layout: the caller turns the responses into its own commands.

use std::sync::Arc;

use egui::{
    Color32, Context, FontId, Galley, Id, Key, LayerId, Order, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetText, pos2, vec2,
};

use super::{DockStyle, PanelLimits, resize::AnchoredResize};

/// Height of a floating window's title bar.
pub const TITLE_HEIGHT: f32 = 14.0;

/// Names of the resize handles, appended to the frame id: `id.with(name)`.
pub const RESIZE_HANDLES: [&str; 8] =
    ["resize", "resize-left", "resize-right", "resize-bottom", "resize-bottom-left", "resize-top", "resize-top-left", "resize-top-right"];

/// Height of the top resize band of a resizable frame. The frame's contents start below it, so
/// the band takes no moves or clicks from a title bar.
pub const RESIZE_BAND: f32 = 4.0;

/// Colors and font of the frame and title bar.
#[derive(Clone, Debug)]
pub struct ChromeStyle {
    /// Painted under the contents, so the window stays opaque when the contents are not.
    pub background: Color32,
    pub stroke: Stroke,
    pub title_background: Color32,
    pub text: Color32,
    pub weak_text: Color32,
    pub accent: Color32,
    /// Fill of a selected or hovered control, such as an icon rail's open item.
    pub active_background: Color32,
    pub font: FontId,
    /// Mirror the title bar: buttons at the left, the title at the right.
    pub right_to_left: bool,
}

impl ChromeStyle {
    pub fn from_ui(ui: &Ui) -> Self {
        Self::from_visuals(ui.visuals())
    }

    /// The colors [`Self::from_dock`] takes from [`DockStyle::from_visuals`], without building
    /// the dock style's labels.
    pub fn from_visuals(visuals: &egui::Visuals) -> Self {
        Self {
            background: visuals.window_fill,
            stroke: visuals.window_stroke,
            title_background: visuals.extreme_bg_color,
            text: visuals.text_color(),
            weak_text: visuals.weak_text_color(),
            accent: visuals.selection.stroke.color,
            active_background: visuals.widgets.active.bg_fill,
            font: FontId::proportional(12.0),
            right_to_left: false,
        }
    }

    pub fn from_dock(style: &DockStyle) -> Self {
        Self {
            background: style.floating_background,
            stroke: style.floating_stroke,
            title_background: style.tab_background,
            text: style.text,
            weak_text: style.inactive_text,
            accent: style.accent,
            active_background: style.active_background,
            font: style.font.clone(),
            right_to_left: style.right_to_left,
        }
    }
}

/// Resize handle hit rects and the edges they move: every side and corner. The top handles
/// fill the band above the contents; the others lie over the contents' edges.
fn handles(rect: Rect) -> [(&'static str, Rect, Vec2); 8] {
    let band_bottom = (rect.top() + RESIZE_BAND).min(rect.bottom());
    let left_corner = (rect.left() + 12.0).min(rect.right());
    let right_corner = (rect.right() - 12.0).max(rect.left());
    [
        ("resize", Rect::from_min_max((rect.max - vec2(12.0, 12.0)).max(rect.min), rect.max), vec2(1.0, 1.0)),
        (
            "resize-left",
            Rect::from_min_max(pos2(rect.left(), rect.top() + 12.0), pos2((rect.left() + 4.0).min(rect.right()), rect.bottom() - 12.0)),
            vec2(-1.0, 0.0),
        ),
        (
            "resize-right",
            Rect::from_min_max(pos2((rect.right() - 4.0).max(rect.left()), rect.top() + 12.0), pos2(rect.right(), rect.bottom() - 12.0)),
            vec2(1.0, 0.0),
        ),
        (
            "resize-bottom",
            Rect::from_min_max(pos2(rect.left() + 12.0, (rect.bottom() - 4.0).max(rect.top())), pos2(rect.right() - 12.0, rect.bottom())),
            vec2(0.0, 1.0),
        ),
        (
            "resize-bottom-left",
            Rect::from_min_max(pos2(rect.left(), (rect.bottom() - 12.0).max(rect.top())), pos2(left_corner, rect.bottom())),
            vec2(-1.0, 1.0),
        ),
        ("resize-top", Rect::from_min_max(pos2(left_corner, rect.top()), pos2(right_corner, band_bottom)), vec2(0.0, -1.0)),
        ("resize-top-left", Rect::from_min_max(rect.min, pos2(left_corner, band_bottom)), vec2(-1.0, -1.0)),
        ("resize-top-right", Rect::from_min_max(pos2(right_corner, rect.top()), pos2(rect.right(), band_bottom)), vec2(1.0, -1.0)),
    ]
}

/// An opaque in-window frame on its own `Order::Middle` layer, `id`, at `rect`.
pub struct FloatingFrame<'a> {
    id: Id,
    rect: Rect,
    bounds: Option<Rect>,
    limits: PanelLimits,
    resizable: bool,
    resize_label: &'a str,
    opacity: f32,
}

/// What a [`FloatingFrame`] did this frame.
pub struct FrameResponse<R> {
    pub inner: R,
    /// Where the frame was drawn, inside its bounds.
    pub rect: Rect,
    pub layer: LayerId,
    /// The rect a resize handle asks for: pointer drags anchored at the opposite edges, or
    /// arrow keys on a focused handle (8 pt per press). Escape and focus loss end a resize.
    pub resized: Option<Rect>,
    /// The handle whose pointer resize started this frame.
    pub resize_started: Option<Id>,
    /// Every handle's response, for focus and gesture bookkeeping.
    pub handles: Vec<Response>,
}

impl<'a> FloatingFrame<'a> {
    pub fn new(id: Id, rect: Rect) -> Self {
        Self { id, rect, bounds: None, limits: PanelLimits::default(), resizable: true, resize_label: "Resize window", opacity: 1.0 }
    }

    /// The frame stays inside `bounds`; the default is the viewport's content rect.
    pub fn bounds(mut self, bounds: Rect) -> Self {
        self.bounds = Some(bounds);
        self
    }

    /// Size limits of a resize, top resize band included. The frame itself is drawn at the
    /// given rect.
    pub fn limits(mut self, limits: PanelLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// Accessibility label of the resize handles.
    pub fn resize_label(mut self, label: &'a str) -> Self {
        self.resize_label = label;
        self
    }

    /// Opacity of everything drawn in the frame, contents included (default 1.0).
    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    pub fn layer(id: Id) -> LayerId {
        LayerId::new(Order::Middle, id)
    }

    /// Draw the frame, then `add_contents` with the rect for its contents (the frame's rect below
    /// the top resize band when resizable), then the outline and handles.
    pub fn show<R>(self, ctx: &Context, style: &ChromeStyle, add_contents: impl FnOnce(&mut Ui, Rect) -> R) -> FrameResponse<R> {
        let bounds = self.bounds.unwrap_or_else(|| ctx.content_rect());
        let rect = clamp(self.rect, bounds);
        let area = egui::Area::new(self.id)
            .fade_in(false)
            .order(Order::Middle)
            .fixed_pos(rect.min)
            .default_size(rect.size())
            .movable(false)
            .constrain_to(bounds);
        let shown = area.show(ctx, |ui| {
            ui.multiply_opacity(self.opacity);
            // Drawn at `rect` itself, also in egui's first (sizing) pass of a new area.
            ui.set_min_size(rect.size());
            ui.allocate_rect(rect, Sense::hover());
            ui.painter().rect_filled(rect, 0.0, style.background);
            let contents =
                if self.resizable { Rect::from_min_max(pos2(rect.left(), (rect.top() + RESIZE_BAND).min(rect.bottom())), rect.max) } else { rect };
            let inner = add_contents(ui, contents);
            ui.painter().rect_stroke(rect, 0.0, style.stroke, StrokeKind::Outside);
            let (resized, resize_started, handles) = if self.resizable { self.handles(ui, rect, bounds, style) } else { (None, None, Vec::new()) };
            FrameResponse { inner, rect, layer: Self::layer(self.id), resized, resize_started, handles }
        });
        shown.inner
    }

    /// Draw the resize handles around `rect` and return `(resized, started, responses)`.
    ///
    /// `resized` is set only for an enabled, uncancelled handle whose clamped next rect differs
    /// from the current one. `started` names the handle whose drag began, which can remain set after
    /// Escape or focus loss cancels that drag. Keyboard arrow steps move an edge by 8 pt; pointer
    /// drags derive the next rect from the cursor relative to the press origin. Limits clamp the
    /// proposal. Stored pointer anchors are removed on cancellation or once the primary button is
    /// released. Handles are accessibility buttons; the diagonal one is painted while hovered,
    /// dragged or focused.
    fn handles(&self, ui: &mut Ui, rect: Rect, bounds: Rect, style: &ChromeStyle) -> (Option<Rect>, Option<Id>, Vec<Response>) {
        let mut resized = None;
        let mut started = None;
        let mut responses = Vec::new();
        let limits = PanelLimits { min: self.limits.min.max(vec2(40.0, RESIZE_BAND + TITLE_HEIGHT + 20.0)).min(self.limits.max), ..self.limits };
        let canceled = ui.input(|input| input.key_pressed(Key::Escape)) || crate::focus::lost(ui.ctx());
        for (name, handle, direction) in handles(rect) {
            if !super::targets::usable(handle) {
                continue;
            }
            let cursor = if direction.x == 0.0 {
                egui::CursorIcon::ResizeVertical
            } else if direction.y == 0.0 {
                egui::CursorIcon::ResizeHorizontal
            } else if direction.x == direction.y {
                egui::CursorIcon::ResizeNwSe
            } else {
                egui::CursorIcon::ResizeNeSw
            };
            let response = ui.interact(handle, self.id.with(name), Sense::drag()).on_hover_cursor(cursor);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), self.resize_label));
            let anchor_id = response.id.with("pointer-anchor");
            crate::layout::drag_focus(ui, &response);
            if response.drag_started()
                && let Some(origin) = ui.input(|i| i.pointer.press_origin())
                && let Some(anchor) = AnchoredResize::new(rect, origin, direction)
            {
                ui.ctx().data_mut(|data| data.insert_temp(anchor_id, anchor));
                started = Some(response.id);
            }
            let mut delta = Vec2::ZERO;
            if response.has_focus() {
                ui.memory_mut(|memory| {
                    memory.set_focus_lock_filter(
                        response.id,
                        egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() },
                    )
                });
                ui.input_mut(|input| {
                    for (key, step) in [
                        (Key::ArrowLeft, vec2(-8.0, 0.0)),
                        (Key::ArrowRight, vec2(8.0, 0.0)),
                        (Key::ArrowUp, vec2(0.0, -8.0)),
                        (Key::ArrowDown, vec2(0.0, 8.0)),
                    ] {
                        if input.consume_key(egui::Modifiers::NONE, key) {
                            delta += step;
                        }
                    }
                });
            }
            let next = if response.dragged() || response.drag_stopped() {
                ui.ctx()
                    .data(|data| data.get_temp::<AnchoredResize>(anchor_id))
                    .and_then(|anchor| ui.ctx().pointer_interact_pos().and_then(|pointer| anchor.rect(pointer, bounds, limits)))
            } else if delta != Vec2::ZERO {
                AnchoredResize::new(rect, rect.max, direction).and_then(|anchor| anchor.rect(rect.max + delta, bounds, limits))
            } else {
                None
            };
            if response.enabled()
                && !canceled
                && let Some(next) = next.filter(|next| *next != rect)
            {
                resized = Some(next);
            }
            if canceled || !ui.input(|input| input.pointer.primary_down()) {
                ui.ctx().data_mut(|data| data.remove::<AnchoredResize>(anchor_id));
            }
            if name == "resize" && (response.hovered() || response.dragged() || response.has_focus()) {
                ui.painter().line_segment([handle.left_bottom(), handle.right_top()], Stroke::new(2.0, style.accent));
            }
            responses.push(response);
        }
        (resized, started, responses)
    }
}

/// A drop preview: a 2 pt accent outline inside `rect`, filled with 15% accent when `filled`.
/// Docking draws its drag ghosts with it, and with
/// [`crate::docking::DockStyle::side_edge_zones`] the region a drop takes; the pane host draws
/// every destination with it.
pub fn drop_preview(painter: &egui::Painter, rect: Rect, accent: Color32, filled: bool) {
    let fill = if filled { accent.gamma_multiply(0.15) } else { Color32::TRANSPARENT };
    painter.rect(rect, 0.0, fill, Stroke::new(2.0, accent), StrokeKind::Inside);
}

/// Keep `rect` inside `bounds`, shrinking it when it is larger.
pub fn clamp(rect: Rect, bounds: Rect) -> Rect {
    if !super::targets::usable(bounds) {
        return rect;
    }
    let size = rect.size().max(vec2(1.0, 1.0)).min(bounds.size());
    let min = rect.min.max(bounds.min).min(bounds.max - size);
    Rect::from_min_size(min, size)
}

/// `text` on one line no wider than `width`, cut at the end with '…'. egui does not wrap a
/// host's pre-laid-out `WidgetText::Galley` again, so that one is laid out again from its job.
pub(super) fn truncated(ui: &Ui, text: WidgetText, width: f32, font: FontId) -> Arc<Galley> {
    let galley = text.into_galley(ui, Some(egui::TextWrapMode::Truncate), width, font);
    if galley.size().x <= width + 0.5 {
        return galley;
    }
    let mut job = (*galley.job).clone();
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    ui.painter().layout_job(job)
}

/// A title bar: an optional title, an optional ▾ menu button and an optional ×, by default 10 pt
/// in from the right edge, mirrored when [`ChromeStyle::right_to_left`] is set. The bar senses
/// clicks and drags; double-clicks are the caller's.
pub struct TitleBar<'a> {
    id: Id,
    move_label: &'a str,
    title: Option<WidgetText>,
    menu: Option<&'a str>,
    close: Option<&'a str>,
    /// The ×'s hit area and glyph, where the caller places them.
    close_at: Option<(Rect, Rect)>,
    movable: bool,
}

/// The parts of a [`TitleBar`] that were shown.
pub struct TitleResponse {
    /// Drag to move; double-click is the caller's (docking minimizes every group).
    pub bar: Response,
    pub menu: Option<Response>,
    pub close: Option<Response>,
}

impl<'a> TitleBar<'a> {
    /// `id` is the bar's own; the buttons use `id.with("menu")` and `id.with("close")`.
    /// `move_label` names the bar for accessibility.
    pub fn new(id: Id, move_label: &'a str) -> Self {
        Self { id, move_label, title: None, menu: None, close: None, close_at: None, movable: true }
    }

    /// Text at the leading edge, cut with '…' to fit.
    pub fn title(mut self, title: impl Into<WidgetText>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Show a ▾ button with this accessibility label.
    pub fn menu(mut self, label: &'a str) -> Self {
        self.menu = Some(label);
        self
    }

    /// Show a × button with this accessibility label.
    pub fn close(mut self, label: &'a str) -> Self {
        self.close = Some(label);
        self
    }

    /// Place the × at `glyph`, taking clicks in `hit`: rects as drawn, already mirrored in a
    /// right-to-left layout. By default the × is a square the bar's height, its center 10 pt
    /// in from the trailing edge.
    pub fn close_at(mut self, hit: Rect, glyph: Rect) -> Self {
        self.close_at = Some((hit, glyph));
        self
    }

    /// Whether the bar shows the grab cursor. It senses drags either way.
    pub fn movable(mut self, movable: bool) -> Self {
        self.movable = movable;
        self
    }

    /// Draw the title bar into `rect` and return its responses.
    ///
    /// `close_at` rects are in logical points, already mirrored in a right-to-left layout; without it
    /// the × is a square the bar's height centered 10 pt in from the trailing edge. Menu and close
    /// buttons reserve space on the trailing side, the draggable bar excludes them, and a title is
    /// truncated to the bar's remaining width. The bar senses clicks and drags, while double-click
    /// behavior (for example minimizing or closing) is the caller's. A right-to-left style mirrors
    /// the layout; `move_label` remains the accessible name of the drag surface. The bar's own id
    /// is used, with `"menu"` and `"close"` child ids for the buttons.
    pub fn show(self, ui: &mut Ui, rect: Rect, style: &ChromeStyle) -> TitleResponse {
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        painter.rect_filled(rect, 0.0, style.title_background);
        let side = rect.height();
        // Laid out left to right, then mirrored in a right-to-left layout.
        let mirror = |r: Rect| {
            if style.right_to_left {
                Rect::from_x_y_ranges((rect.left() + rect.right() - r.right())..=(rect.left() + rect.right() - r.left()), r.y_range())
            } else {
                r
            }
        };
        let close_rect = self.close.map(|_| match self.close_at {
            Some((hit, _)) => mirror(hit),
            None => Rect::from_center_size(pos2(rect.right() - 10.0, rect.center().y), vec2(side, side)),
        });
        let menu_right = close_rect.map_or(rect.right() - 3.0, |close| close.left() - 2.0);
        let menu_rect = self.menu.map(|_| Rect::from_min_max(pos2(menu_right - side, rect.top()), pos2(menu_right, rect.bottom())));
        let buttons = menu_rect.or(close_rect).map_or(rect.right(), |r| r.left());
        let bar_rect = mirror(Rect::from_min_max(rect.min, pos2(buttons.max(rect.left()), rect.bottom())));
        let (close_rect, menu_rect) = (close_rect.map(mirror), menu_rect.map(mirror));
        let bar = ui.interact(bar_rect, self.id, Sense::click_and_drag());
        let bar = if self.movable { bar.on_hover_cursor(egui::CursorIcon::Grab) } else { bar };
        bar.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), self.move_label));
        if let Some(title) = self.title {
            let galley = truncated(ui, title, (bar_rect.width() - 12.0).max(0.0), style.font.clone());
            let x = if style.right_to_left { bar_rect.right() - 6.0 - galley.size().x } else { bar_rect.left() + 6.0 };
            painter.galley(pos2(x, rect.center().y - galley.size().y * 0.5), galley, style.text);
        }
        if bar.has_focus() {
            painter.rect_stroke(bar_rect.shrink(1.0), 0.0, Stroke::new(1.0, style.accent), StrokeKind::Inside);
        }
        let button = |ui: &mut Ui, rect: Rect, name: &str, label: &str| {
            let response = ui.interact(rect, self.id.with(name), Sense::click());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
            let color = if response.hovered() || response.has_focus() { style.text } else { style.weak_text };
            (response, Stroke::new(1.2, color))
        };
        let menu = menu_rect.zip(self.menu).map(|(menu_rect, label)| {
            let (response, stroke) = button(ui, menu_rect, "menu", label);
            let c = menu_rect.center();
            painter.add(egui::Shape::convex_polygon(vec![c + vec2(-3.5, -1.5), c + vec2(3.5, -1.5), c + vec2(0.0, 2.5)], stroke.color, Stroke::NONE));
            response
        });
        let close = close_rect.zip(self.close).map(|(close_rect, label)| {
            let (response, stroke) = button(ui, close_rect, "close", label);
            let r = self.close_at.map_or_else(|| Rect::from_center_size(close_rect.center(), Vec2::splat((side * 0.5).max(4.0))), |(_, glyph)| glyph);
            painter.line_segment([r.left_top(), r.right_bottom()], stroke);
            painter.line_segment([r.right_top(), r.left_bottom()], stroke);
            response
        });
        TitleResponse { bar, menu, close }
    }
}

/// One step of a title-bar move, from [`live_move`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MoveStep {
    /// The rect follows the pointer, inside the bounds.
    Moving(Rect),
    /// Escape or window focus loss ended the move: the rect it started from.
    Canceled(Rect),
    /// Released over `pointer`, with the rect where it was dropped.
    Dropped { rect: Rect, pointer: Pos2 },
}

/// Move `rect` with a drag on `bar` (usually [`TitleResponse::bar`]). The rect and the press
/// position at drag start are kept in egui memory under the bar's id, so the window follows the
/// pointer without drift. Escape or window focus loss ends the move and returns the start rect;
/// the rest of that drag is ignored. `None` when the bar is not being moved.
pub fn live_move(ctx: &Context, bar: &Response, rect: Rect, bounds: Rect) -> Option<MoveStep> {
    let key = bar.id.with("live-move");
    if bar.drag_started()
        && let Some(origin) = ctx.input(|input| input.pointer.press_origin())
    {
        ctx.data_mut(|data| data.insert_temp(key, (rect, origin)));
    }
    let (start, origin) = ctx.data(|data| data.get_temp::<(Rect, Pos2)>(key))?;
    let forget = || ctx.data_mut(|data| data.remove::<(Rect, Pos2)>(key));
    if ctx.input(|input| input.key_pressed(Key::Escape)) || crate::focus::lost(ctx) {
        forget();
        return Some(MoveStep::Canceled(start));
    }
    let pointer = ctx.pointer_latest_pos().unwrap_or(origin);
    let moved = clamp(start.translate(pointer - origin), bounds);
    if bar.drag_stopped() {
        forget();
        return Some(MoveStep::Dropped { rect: moved, pointer });
    }
    if bar.dragged() {
        return Some(MoveStep::Moving(moved));
    }
    // The drag ended without a release on this bar (the bar was not shown).
    forget();
    None
}
