use std::{fmt::Debug, hash::Hash};

use egui::{Color32, Context, FontId, Id, Key, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2};

use crate::{
    layout::{ResizeHandle, ResizeHandleStyle, SplitAxis, SplitSize, split_rect},
    tabs,
};

use super::{Action, DockError, DropTarget, Layout, Node, Permissions, Placement, resolve_drop};

/// Application-supplied chrome. Body renderers retain their own UI style and contents.
#[derive(Clone, Debug)]
pub struct DockStyle {
    pub tab_height: f32,
    pub gap: f32,
    pub min_pane: f32,
    pub background: Color32,
    pub tab_background: Color32,
    pub active_background: Color32,
    pub text: Color32,
    pub inactive_text: Color32,
    pub border: Stroke,
    pub accent: Color32,
    pub font: FontId,
    /// Localize these action labels in the application adapter.
    pub float_label: String,
    pub close_label: String,
    pub move_label: String,
    pub panels_label: String,
    pub resize_label: String,
    pub resize_window_label: String,
}

impl DockStyle {
    pub fn from_ui(ui: &Ui) -> Self {
        let visuals = ui.visuals();
        Self {
            tab_height: 26.0,
            gap: 4.0,
            min_pane: 40.0,
            background: visuals.panel_fill,
            tab_background: visuals.extreme_bg_color,
            active_background: visuals.widgets.active.bg_fill,
            text: visuals.text_color(),
            inactive_text: visuals.weak_text_color(),
            border: visuals.widgets.noninteractive.bg_stroke,
            accent: visuals.selection.stroke.color,
            font: FontId::proportional(12.0),
            float_label: "Float panel".into(),
            close_label: "Close panel".into(),
            move_label: "Move to group".into(),
            panels_label: "Panels".into(),
            resize_label: "Resize panels".into(),
            resize_window_label: "Resize panel window".into(),
        }
    }

    fn valid(&self) -> bool {
        [self.tab_height, self.gap, self.min_pane, self.border.width, self.font.size].iter().all(|v| v.is_finite() && (0.0..=4096.0).contains(v))
            && self.tab_height > 0.0
            && self.font.size > 0.0
    }
}

#[derive(Clone, Debug)]
pub struct DockOutput<P> {
    pub actions: Vec<Action<P>>,
    pub error: Option<DockError>,
    pub focused: Option<P>,
    /// Whole-floating-group gesture identity; host may display its own group drop preview.
    pub group_drag: Option<P>,
    /// Whole-group release for host-owned atomic docking policy.
    pub group_drop: Option<(P, Pos2)>,
}

impl<P> Default for DockOutput<P> {
    fn default() -> Self {
        Self { actions: Vec::new(), error: None, focused: None, group_drag: None, group_drop: None }
    }
}

/// Preferred body bounds. Tiny viewports relax minima proportionally rather than producing
/// inverted rectangles. Maximums constrain a pane when its sibling can occupy the remainder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelLimits {
    pub min: Vec2,
    pub max: Vec2,
}

impl Default for PanelLimits {
    fn default() -> Self {
        Self { min: Vec2::ZERO, max: Vec2::splat(1_000_000.0) }
    }
}

impl PanelLimits {
    fn valid(self) -> bool {
        self.min.is_finite()
            && self.max.is_finite()
            && self.min.x >= 0.0
            && self.min.y >= 0.0
            && self.max.x >= self.min.x
            && self.max.y >= self.min.y
            && self.max.x <= 1_000_000.0
            && self.max.y <= 1_000_000.0
    }
}

#[derive(Clone)]
struct Drag<P> {
    panel: P,
    origin: Pos2,
    source: Rect,
    grab: Vec2,
}

#[derive(Clone)]
struct DragOpacity {
    from: f32,
    target: f32,
    started: f64,
    current: f32,
}

#[derive(Clone)]
struct GroupDrag<P> {
    keep_cursor: bool,
    origin: Pos2,
    panel: P,
    original: [f32; 4],
    members: Vec<P>,
    selected: P,
}

/// Start a docking drag from an application's existing tab/header. Use the same area ID when
/// switching from a legacy layout to [`DockArea`]. The model is untouched until a valid release.
pub fn begin_drag<P: Clone + Send + Sync + 'static>(ctx: &Context, area_id: Id, panel: P, source_rect: Rect) -> bool {
    let Some(origin) = ctx.pointer_interact_pos().filter(|p| p.is_finite()) else { return false };
    if !super::targets::usable(source_rect) {
        return false;
    }
    let grab = (origin - source_rect.min).clamp(Vec2::ZERO, source_rect.size());
    ctx.data_mut(|d| d.insert_temp(area_id.with("drag"), Drag { panel, origin, source: source_rect, grab }));
    true
}

/// Whether this area's authoritative tab/group gesture is active, independent of its display layout.
pub fn is_dragging<P: Clone + Send + Sync + 'static>(ctx: &Context, area_id: Id) -> bool {
    ctx.data(|data| data.get_temp::<Drag<P>>(area_id.with("drag")).is_some() || data.get_temp::<GroupDrag<P>>(area_id.with("group-drag")).is_some())
}

/// Discard pending tab/group gestures and an actively dragged renderer-owned control before
/// replacing a workspace. Body/custom-header/menu gestures and other areas are untouched.
/// Ownership is recorded as controls render; before an initial render or drag threshold there
/// may be no active egui drag to cancel. Keyboard focus is unchanged. The host owns the
/// replacement snapshot, so this does not restore a group's previous rectangle.
pub fn cancel_drag<P: Clone + Send + Sync + 'static>(ctx: &Context, area_id: Id) {
    let dragged = ctx.dragged_id();
    let owned = ctx.data_mut(|d| {
        d.remove::<DragOpacity>(area_id.with("drag-opacity"));
        d.remove::<Drag<P>>(area_id.with("drag"));
        d.remove::<GroupDrag<P>>(area_id.with("group-drag"));
        let ids = d.remove_temp::<Vec<Id>>(area_id.with("owned-interactions"));
        let resize = d.get_temp::<Id>(area_id.with("pointer-resize"));
        d.remove::<Id>(area_id.with("pointer-resize"));
        if let Some(id) = resize {
            d.remove::<super::AnchoredResize>(id.with("pointer-anchor"));
        }
        if let Some(ids) = &ids {
            for id in ids {
                d.remove::<super::AnchoredResize>(id.with("pointer-anchor"));
            }
        }
        dragged.is_some_and(|id| resize == Some(id) || ids.is_some_and(|ids| ids.contains(&id)))
    });
    if owned {
        ctx.stop_dragging();
    }
}

/// Host-owned drop resolution. Returning feedback consumes the gesture, including no-op
/// targets. Hosts commit their staged transaction on release; the renderer emits no move.
pub struct DropRequest<'a, P> {
    pub panel: &'a P,
    pub pointer: Pos2,
    pub targets: &'a [DropTarget<P>],
    pub floating_rect: Option<Rect>,
    pub whole_group: bool,
    pub released: bool,
    pub workspace: Rect,
    pub screen: Rect,
}

#[derive(Clone, Debug, Default)]
pub struct DropFeedback {
    pub preview: Option<Rect>,
    pub insertion: Option<Rect>,
}

/// One mutable application adapter owns bodies, menus and shaped text, avoiding competing
/// mutable closures. Logical labels remain separate and are used for tab accessibility.
pub trait DockContent<P> {
    fn body(&mut self, ui: &mut Ui, panel: &P);
    fn resolve_drag(&mut self, _ctx: &Context, _style: &DockStyle, _request: &DropRequest<'_, P>) -> Option<DropFeedback> {
        None
    }

    /// Optional display-only arrangement prepared before bodies render. Interaction targets
    /// and emitted actions continue to use the authoritative layout passed to DockArea.
    /// Called on every surface; request is absent when there is no active gesture.
    fn preview_layout(&mut self, _ctx: &Context, _style: &DockStyle, _request: Option<&DropRequest<'_, P>>) -> Option<Layout<P>> {
        None
    }

    /// Measured display targets, for aligning feedback with a temporary layout. These
    /// never replace authoritative targets used to resolve the pointer.
    fn preview_targets(&mut self, _ctx: &Context, _targets: &[DropTarget<P>]) {}
    /// Paint host-owned display feedback after the moving UI, independently of its opacity.
    fn paint_preview(&mut self, _ctx: &Context) {}

    /// Display-only docking geometry transition in seconds. Zero keeps instant sizing.
    fn preview_geometry_duration(&self) -> f32 {
        0.0
    }
    /// Change this when replacing the workspace to discard unrelated transition history.
    fn preview_geometry_generation(&self) -> u64 {
        0
    }
    /// Optional starting bounds for a panel newly appearing on this surface, e.g. a host's
    /// previously separate document area. Existing transition history takes precedence.
    fn preview_geometry_origin(&self, _ctx: &Context, _panel: &P) -> Option<Rect> {
        None
    }
    /// Animated group bounds and their measured destination, including return/settling frames.
    fn preview_geometry(&mut self, _ctx: &Context, _panels: &[P], _display: Rect, _destination: Rect) {}

    /// Opt into a stationary receiving header by returning its authoritative target snapshot.
    /// Only header geometry/order is overridden; bodies retain measured preview bounds.
    /// The snapshot's anchor must belong to the receiving group (not the incoming group).
    fn preview_tab_strip(&self, _ctx: &Context, _panels: &[P]) -> Option<DropTarget<P>> {
        None
    }

    fn label(&mut self, _ui: &Ui, _panel: &P, logical: &str, _font: &FontId, _color: Color32) -> egui::WidgetText {
        logical.into()
    }
    /// Shape stock Float/Move/Close action text independently of panel identity.
    /// The renderer preserves `logical` for accessibility, even when the visual text differs.
    fn action_label(&mut self, _ui: &Ui, logical: &str, _font: &FontId, _color: Color32) -> egui::WidgetText {
        logical.into()
    }
    /// Full rendered group bounds, including its tab header.
    /// Opt into transient UI dragging without moving the saved source layout.
    fn live_drag(&self) -> bool {
        false
    }
    fn drag_members(&self, _ctx: &Context, _panel: &P) -> Option<Vec<P>> {
        None
    }
    fn render_drag_ui(&self) -> bool {
        true
    }
    /// Opacity of the transient moving UI only; destination feedback stays unaffected.
    /// Non-finite values fall back to opaque and finite values are clamped to 0..=1.
    fn drag_opacity(&self, _ctx: &Context, _request: &DropRequest<'_, P>) -> f32 {
        1.0
    }
    /// Transition duration in seconds for moving-UI opacity. Zero preserves immediate changes.
    fn drag_opacity_duration(&self) -> f32 {
        0.0
    }
    /// Preferred floating size, independent of the current dock geometry.
    fn floating_size(&self, _panel: &P) -> Option<Vec2> {
        None
    }
    /// Publish measured targets for hosts coordinating multiple docking surfaces.
    fn drop_targets(&mut self, _ctx: &Context, _targets: &[DropTarget<P>]) {}
    fn group_rect(&mut self, _ctx: &Context, _panels: &[P], _rect: Rect) {}
    fn panel_menu(&mut self, _ui: &mut Ui, _panel: &P) {}
    /// Opt into docked empty-header/top-border whole-group gestures. Hosts consume group drops.
    fn drag_docked_headers(&self) -> bool {
        false
    }
    fn group_header_width(&self, _panels: &[P], _active: &P, _floating: bool) -> f32 {
        0.0
    }
    fn group_header(&mut self, _ui: &mut Ui, _panels: &[P], _active: &P, _floating: bool) {}
}

struct BodyContent<F>(F);
impl<P, F: FnMut(&mut Ui, &P)> DockContent<P> for BodyContent<F> {
    fn body(&mut self, ui: &mut Ui, panel: &P) {
        (self.0)(ui, panel);
    }
}

/// Optional renderer for applications without an existing docking surface. Apps with custom
/// chrome can instead use the model, target resolver and resize/tab primitives independently.
pub struct DockArea<P = ()> {
    id: Id,
    surface: Id,
    activation: Option<P>,
    blockers: Vec<Rect>,
}

impl<P: Clone + Eq + Hash + Debug + Send + Sync + 'static> DockArea<P> {
    pub fn new(id: Id) -> Self {
        Self { id, surface: id, activation: None, blockers: Vec::new() }
    }

    /// Independent divider identity for multiple surfaces sharing a drag session.
    pub fn surface(mut self, id: Id) -> Self {
        self.surface = id;
        self
    }

    /// Whether keyboard focus belongs to a rendered docking control in this layout.
    /// Use the same [`Self::surface`] configuration as rendering when querying focus.
    /// Panel bodies and application-provided header/menu widgets are excluded. Hosts may
    /// route nonconflicting global shortcuts through chrome while retaining widget ownership
    /// of activation, traversal, resize and cancellation keys. Invalid layouts return false.
    pub fn chrome_has_focus(&self, ctx: &egui::Context, layout: &Layout<P>) -> bool {
        let Some(focused) = ctx.memory(|memory| memory.focused()) else { return false };
        if layout.validate().is_err() || ctx.read_response(focused).is_none() {
            return false;
        }
        let matches = |id: Id| id == focused;
        let group = |panels: &[P]| {
            panels.iter().any(|panel| matches(self.id.with(("tab", panel))) || matches(self.id.with(("overflow", panel))))
                || ["move-group", "move-group-header", "move-group-border"].iter().any(|name| matches(self.id.with((*name, panels.first()))))
        };
        for floating in &layout.floating {
            if group(&floating.panels)
                || floating.panels.first().is_some_and(|first| {
                    ["resize", "resize-left", "resize-right", "resize-bottom", "resize-bottom-left"]
                        .iter()
                        .any(|name| matches(self.id.with(("floating", first)).with(*name)))
                })
            {
                return true;
            }
        }
        let mut nodes = layout.root.as_ref().map(|root| vec![(root, Vec::<bool>::new())]).unwrap_or_default();
        while let Some((node, path)) = nodes.pop() {
            match node {
                Node::Tabs { panels, .. } if group(panels) => return true,
                Node::Stack { entries } => {
                    if entries.iter().any(|entry| {
                        matches(self.id.with(("tab", &entry.panel)))
                            || matches(self.id.with(("collapse", &entry.panel)))
                            || matches(self.id.with(("stack-resize", &entry.panel)))
                    }) {
                        return true;
                    }
                }
                Node::Split { first, second, .. } => {
                    if matches(self.surface.with(("split", &path))) {
                        return true;
                    }
                    let mut second_path = path.clone();
                    second_path.push(true);
                    let mut first_path = path;
                    first_path.push(false);
                    nodes.push((second, second_path));
                    nodes.push((first, first_path));
                }
                _ => {}
            }
        }
        false
    }

    /// Raise and focus a panel's owning floating area on this frame. Apply Activate through
    /// the application's command path as well to select its tab.
    pub fn raise_panel(mut self, panel: P) -> Self {
        self.activation = Some(panel);
        self
    }

    /// Host overlays that must reject docking gestures, including floating fallback.
    pub fn blocked_rects(mut self, rects: &[Rect]) -> Self {
        self.blockers = rects.to_vec();
        self
    }

    /// Render a validated snapshot and return requested actions for the app's command handler.
    /// No action is applied here. Panel contents get a global stable egui ID across moves.
    /// Floating groups are egui areas inside this viewport; this does not create native windows.
    pub fn show(
        self,
        ui: &mut Ui,
        layout: &Layout<P>,
        style: &DockStyle,
        label: impl Fn(&P) -> String,
        permissions: impl Fn(&P) -> Permissions,
        body: impl FnMut(&mut Ui, &P),
    ) -> DockOutput<P> {
        self.show_with_limits(ui, layout, style, label, permissions, |_| PanelLimits::default(), body)
    }

    /// Like [`Self::show`], with per-panel size policies supplied by the application.
    #[allow(clippy::too_many_arguments)]
    pub fn show_with_limits(
        self,
        ui: &mut Ui,
        layout: &Layout<P>,
        style: &DockStyle,
        label: impl Fn(&P) -> String,
        permissions: impl Fn(&P) -> Permissions,
        limits: impl Fn(&P) -> PanelLimits,
        body: impl FnMut(&mut Ui, &P),
    ) -> DockOutput<P> {
        self.show_customized(ui, layout, style, label, permissions, limits, &mut BodyContent(body))
    }

    /// Render with application-owned chrome and shaped labels. Header callbacks queue app
    /// commands; structural changes are applied after rendering, like DockOutput actions.
    #[allow(clippy::too_many_arguments)]
    pub fn show_customized(
        self,
        ui: &mut Ui,
        layout: &Layout<P>,
        style: &DockStyle,
        label: impl Fn(&P) -> String,
        permissions: impl Fn(&P) -> Permissions,
        limits: impl Fn(&P) -> PanelLimits,
        content: &mut impl DockContent<P>,
    ) -> DockOutput<P> {
        let rect = ui.available_rect_before_wrap();
        if let Err(error) = layout.validate() {
            cancel_drag::<P>(ui.ctx(), self.id);
            return DockOutput { error: Some(error), ..Default::default() };
        }
        if !style.valid() || !super::targets::usable(rect) || layout.panels().into_iter().any(|p| !limits(p).valid()) {
            cancel_drag::<P>(ui.ctx(), self.id);
            return DockOutput { error: Some(DockError::InvalidGeometry), ..Default::default() };
        }
        let strips = super::strips::Strips::load(ui.ctx(), self.surface, content.preview_geometry_generation());
        let mut renderer = Renderer {
            id: self.id,
            surface: self.surface,
            style,
            label: &label,
            permissions: &permissions,
            limits: &limits,
            content,
            activation: self.activation,
            blockers: self.blockers,
            layout,
            targets: Vec::new(),
            owned_interactions: Vec::new(),
            output: DockOutput::default(),
            workspace: rect,
            previewing: false,
            geometry: None,
            strips: Some(strips),
            headers: Vec::new(),
        };
        let targets_key = self.surface.with("authoritative-targets");
        let cached = ui.ctx().data(|d| d.get_temp::<Vec<DropTarget<P>>>(targets_key)).unwrap_or_default();
        renderer.targets = cached.clone();
        let preview = renderer
            .prepare_preview(ui.ctx())
            .filter(|layout| layout.validate().is_ok() && layout.panels().into_iter().all(|panel| limits(panel).valid()));
        let display = preview.as_ref().unwrap_or(layout);
        renderer.previewing = preview.is_some();
        let duration = renderer.content.preview_geometry_duration();
        if duration.is_finite() && duration > 0.0 {
            let measured = measure_layout(display, rect, ui.ctx().content_rect(), style, &permissions, &limits).unwrap_or_default();
            let moving = renderer.moving_panels(ui.ctx());
            let origins: Vec<_> = measured
                .iter()
                .filter_map(|(panel, _)| {
                    renderer
                        .content
                        .preview_geometry_origin(ui.ctx(), panel)
                        .filter(|rect| super::targets::usable(*rect))
                        .map(|rect| (panel.clone(), rect))
                })
                .collect();
            renderer.geometry = Some(super::geometry::Geometry::prepare(
                ui.ctx(),
                self.surface,
                super::geometry::Options { generation: renderer.content.preview_geometry_generation(), duration },
                preview.is_some(),
                &measured,
                &moving,
                &origins,
            ));
        } else {
            ui.ctx().data_mut(|data| data.remove::<super::geometry::Geometry<P>>(self.surface.with("preview-geometry")));
        }
        let presenting = preview.is_some()
            || renderer.geometry.as_ref().is_some_and(|state| state.active)
            || renderer.strips.as_ref().is_some_and(|state| state.active());
        let dragging = renderer.content.live_drag()
            && ui
                .ctx()
                .data(|d| d.get_temp::<Drag<P>>(self.id.with("drag")).is_some() || d.get_temp::<GroupDrag<P>>(self.id.with("group-drag")).is_some());
        renderer.targets.clear();
        if presenting {
            ui.painter().rect_filled(rect, 0.0, style.background);
        }
        if let Some(root) = &display.root {
            ui.scope(|ui| {
                if presenting {
                    ui.visuals_mut().disabled_alpha = 1.0;
                    ui.disable();
                }
                renderer.node(ui, root, rect, &mut Vec::new());
            });
            ui.allocate_rect(rect, Sense::hover());
        }
        renderer.floating(ui.ctx(), display, presenting);
        renderer.header_overlays(ui.ctx());
        if preview.is_some() {
            renderer.content.preview_targets(ui.ctx(), &renderer.targets);
        }
        if (dragging || presenting) && !cached.is_empty() {
            renderer.targets = cached;
        } else if !presenting {
            ui.ctx().data_mut(|d| d.insert_temp(targets_key, renderer.targets.clone()));
        }
        renderer.content.drop_targets(ui.ctx(), &renderer.targets);
        renderer.drag_ui(ui.ctx());
        renderer.content.paint_preview(ui.ctx());
        if presenting {
            renderer.output.actions.clear();
            renderer.output.focused = None;
        }
        renderer.finish_group_drag(ui.ctx());
        renderer.finish_drag(ui.ctx());
        if let Some(strips) = renderer.strips.take() {
            strips.finish(ui.ctx(), self.surface);
        }
        ui.ctx().data_mut(|d| d.insert_temp(self.id.with("owned-interactions"), renderer.owned_interactions));
        renderer.output
    }
}

struct Renderer<'a, P> {
    workspace: Rect,
    previewing: bool,
    geometry: Option<super::geometry::Geometry<P>>,
    strips: Option<super::strips::Strips<P>>,
    headers: Vec<HeaderJob<P>>,
    id: Id,
    surface: Id,
    style: &'a DockStyle,
    label: &'a dyn Fn(&P) -> String,
    permissions: &'a dyn Fn(&P) -> Permissions,
    limits: &'a dyn Fn(&P) -> PanelLimits,
    content: &'a mut dyn DockContent<P>,
    activation: Option<P>,
    blockers: Vec<Rect>,
    layout: &'a Layout<P>,
    targets: Vec<DropTarget<P>>,
    owned_interactions: Vec<Id>,
    output: DockOutput<P>,
}

struct HeaderJob<P> {
    panels: Vec<P>,
    active: usize,
    rect: Rect,
    floating: bool,
    frozen: Option<DropTarget<P>>,
}

impl<P: Clone + Eq + Hash + Debug + Send + Sync + 'static> Renderer<'_, P> {
    fn presentation_rect(&mut self, ctx: &Context, panels: &[P], destination: Rect) -> Rect {
        let display = self.geometry.as_ref().map(|state| state.rect(panels, destination)).unwrap_or(destination);
        if display != destination {
            self.content.preview_geometry(ctx, panels, display, destination);
        }
        display
    }

    fn register_interaction(&mut self, ctx: &Context, id: Id) {
        // A validated tree has at most 2*MAX_PANELS nodes. Tabs, stack dividers,
        // floating grips/corners and splits fit comfortably inside this fixed bound.
        const CAP: usize = 16 * super::MAX_PANELS;
        if self.owned_interactions.len() < CAP && !self.owned_interactions.contains(&id) {
            self.owned_interactions.push(id);
        }
        // Retain prior-frame controls until return, and publish current controls before
        // app callbacks. No Context lock spans a callback or a stop_dragging call.
        ctx.data_mut(|d| {
            let ids = d.get_temp_mut_or_default::<Vec<Id>>(self.id.with("owned-interactions"));
            if ids.len() < CAP && !ids.contains(&id) {
                ids.push(id);
            }
        });
    }

    fn node(&mut self, ui: &mut Ui, node: &Node<P>, rect: Rect, path: &mut Vec<bool>) {
        if !super::targets::usable(rect) {
            return;
        }
        match node {
            Node::Split { axis, size, first, second } => {
                let Some((split, lo, hi)) = constrained_split(rect, *axis, *size, self.style.gap, self.node_limits(first), self.node_limits(second))
                else {
                    return;
                };
                path.push(false);
                self.node(ui, first, split.first, path);
                path.pop();
                path.push(true);
                self.node(ui, second, split.second, path);
                path.pop();
                if self.geometry.as_ref().is_some_and(|state| state.active) {
                    return;
                }
                // Opted-in group headers own their top border. Keep the vertical
                // splitter inside its divider instead of stealing that drag band.
                let response = ResizeHandle::new(
                    self.surface.with(("split", &*path)),
                    *axis,
                    split.divider,
                    &self.style.resize_label,
                    ResizeHandleStyle { hovered: self.style.accent, dragged: self.style.accent, focused: self.style.accent },
                )
                .hit_margin(if *axis == SplitAxis::Vertical && self.content.drag_docked_headers() { 0.0 } else { 3.0 })
                .value(split.first_size(), lo..=hi)
                .show(ui);
                self.register_interaction(ui.ctx(), response.response.id);
                if response.delta != 0.0
                    && let Some(size) = split.resized(*size, (split.first_size() + response.delta).clamp(lo, hi) - split.first_size(), 0.0)
                {
                    self.output.actions.push(Action::ResizeSplit { path: path.clone(), size });
                }
            }
            Node::Tabs { panels, active } => self.group(ui, panels, *active, rect, false, rect),
            Node::Stack { entries } => {
                let total_header = (entries.len() as f32 * self.style.tab_height).min(rect.height());
                let remaining = (rect.height() - total_header).max(0.0);
                let total_wanted: f64 = entries.iter().filter(|e| e.open).map(|e| f64::from(e.height.unwrap_or(self.style.min_pane).max(1.0))).sum();
                let header = self.style.tab_height.min(rect.height() / entries.len().max(1) as f32);
                let mut top = rect.top();
                let mut dividers = Vec::new();
                for (index, entry) in entries.iter().enumerate() {
                    let share = if entry.open && total_wanted > 0.0 {
                        (f64::from(remaining) * f64::from(entry.height.unwrap_or(self.style.min_pane).max(1.0)) / total_wanted) as f32
                    } else {
                        0.0
                    };
                    let panel_rect = Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), (top + header + share).min(rect.bottom())));
                    let next_top = panel_rect.bottom();
                    let panel_rect = self.presentation_rect(ui.ctx(), std::slice::from_ref(&entry.panel), panel_rect);
                    self.content.group_rect(ui.ctx(), std::slice::from_ref(&entry.panel), panel_rect);
                    let title_rect =
                        Rect::from_min_max(panel_rect.min, pos2(panel_rect.right(), (panel_rect.top() + header).min(panel_rect.bottom())));
                    self.tab(ui, &entry.panel, title_rect, entry.open, panel_rect);
                    let collapse = Rect::from_min_max(pos2((title_rect.right() - 18.0).max(title_rect.left()), title_rect.top()), title_rect.max);
                    let response = ui.interact(collapse, self.id.with(("collapse", &entry.panel)), Sense::click());
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), entry.open, (self.label)(&entry.panel))
                    });
                    ui.painter().with_clip_rect(collapse).text(
                        collapse.center(),
                        egui::Align2::CENTER_CENTER,
                        if entry.open { "−" } else { "+" },
                        self.style.font.clone(),
                        self.style.text,
                    );
                    if response.clicked() {
                        self.output.actions.push(Action::SetStackOpen { panel: entry.panel.clone(), open: !entry.open });
                    }
                    let body_rect = Rect::from_min_max(pos2(panel_rect.left(), title_rect.bottom()), panel_rect.max);
                    if entry.open {
                        self.contents(ui, &entry.panel, body_rect);
                    }
                    self.targets.push(DropTarget {
                        anchor: entry.panel.clone(),
                        rect: panel_rect,
                        tab_strip: Some(title_rect),
                        tabs: vec![(entry.panel.clone(), title_rect)],
                        allow_splits: true,
                        accepts_tabs: (self.permissions)(&entry.panel).accepts_tabs,
                    });
                    top = next_top;
                    if entry.open && remaining > 0.0 {
                        dividers.push((index, Rect::from_min_max(pos2(rect.left(), top - 1.0), pos2(rect.right(), top + 1.0)).intersect(rect)));
                    }
                }
                // Register dividers after the headers so their narrow hit regions win at the
                // shared boundary. Closed entries retain their saved sizes and are skipped.
                if self.geometry.as_ref().is_some_and(|state| state.active) {
                    return;
                }
                for (index, divider) in dividers {
                    let Some(first) = entries.get(index) else { continue };
                    let Some(second) = entries.iter().skip(index.saturating_add(1)).find(|entry| entry.open) else { continue };
                    let a = f64::from(first.height.unwrap_or(self.style.min_pane).max(1.0));
                    let b = f64::from(second.height.unwrap_or(self.style.min_pane).max(1.0));
                    let scale = f64::from(remaining) / total_wanted;
                    let first_size = (a * scale) as f32;
                    let pair_size = ((a + b) * scale) as f32;
                    let body_limits = |panel: &P| {
                        let mut limits = (self.limits)(panel);
                        limits.min.y = limits.min.y.max(self.style.min_pane);
                        limits.max = limits.max.max(limits.min);
                        limits
                    };
                    let (lo, hi) = split_bounds(pair_size, SplitAxis::Vertical, body_limits(&first.panel), body_limits(&second.panel));
                    let response = ResizeHandle::new(
                        self.id.with(("stack-resize", &first.panel)),
                        SplitAxis::Vertical,
                        divider,
                        &self.style.resize_label,
                        ResizeHandleStyle { hovered: self.style.accent, dragged: self.style.accent, focused: self.style.accent },
                    )
                    .value(first_size, lo..=hi)
                    .show(ui);
                    self.register_interaction(ui.ctx(), response.response.id);
                    if response.delta != 0.0 && scale > 0.0 {
                        let delta = f64::from((first_size + response.delta).clamp(lo, hi) - first_size) / scale;
                        let next_a = (a + delta).clamp((a + b - 1_000_000.0).max(1.0), (a + b - 1.0).min(1_000_000.0));
                        self.output.actions.push(Action::ResizeStack { panel: first.panel.clone(), height: Some(next_a as f32) });
                        self.output.actions.push(Action::ResizeStack { panel: second.panel.clone(), height: Some((a + b - next_a) as f32) });
                    }
                }
            }
        }
    }

    fn node_limits(&self, node: &Node<P>) -> PanelLimits {
        node_limits(node, self.style, &self.permissions, &self.limits)
    }

    fn moving_panels(&self, ctx: &Context) -> Vec<P> {
        if !self.content.live_drag() {
            return Vec::new();
        }
        if let Some(drag) = ctx.data(|d| d.get_temp::<Drag<P>>(self.id.with("drag"))) {
            return self.content.drag_members(ctx, &drag.panel).unwrap_or_else(|| vec![drag.panel]);
        }
        if let Some(drag) = ctx.data(|d| d.get_temp::<GroupDrag<P>>(self.id.with("group-drag"))) {
            return self.content.drag_members(ctx, &drag.panel).or_else(|| Some(drag.members)).unwrap_or_default();
        }
        Vec::new()
    }

    fn contents(&mut self, ui: &mut Ui, panel: &P, rect: Rect) {
        if !super::targets::usable(rect) {
            return;
        }
        ui.painter().rect_filled(rect, 0.0, self.style.background);
        let moving = self.moving_panels(ui.ctx()).contains(panel);
        if moving {
            return;
        }
        let mut child =
            ui.new_child(UiBuilder::new().id(self.id.with(("body", panel))).max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        self.content.body(&mut child, panel);
        if child.rect_contains_pointer(rect) && child.input(|i| i.pointer.primary_pressed()) {
            self.output.focused = Some(panel.clone());
        }
    }

    fn move_floating_group(&mut self, ui: &mut Ui, panels: &[P], selected: &P, rect: Rect, hit: Rect, id: Id) {
        if !super::targets::usable(hit) {
            return;
        }
        let response = ui.interact(hit, id, Sense::drag());
        let response = if id == self.id.with(("move-group-border", panels.first())) {
            response.on_hover_cursor(egui::CursorIcon::Default)
        } else {
            response.on_hover_cursor(egui::CursorIcon::Grab)
        };
        self.register_interaction(ui.ctx(), response.id);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &self.style.move_label));
        if panels.iter().all(|p| (self.permissions)(p).movable)
            && response.drag_started()
            && let Some(first) = panels.first()
        {
            let origin = ui.input(|input| input.pointer.press_origin()).unwrap_or(hit.center());
            let rect = if self.layout.floating.iter().any(|group| group.panels.contains(first)) {
                rect
            } else {
                let size = self.content.floating_size(selected).filter(|s| s.is_finite() && s.x > 0.0 && s.y > 0.0).unwrap_or(rect.size());
                let grab = (origin - rect.min).clamp(Vec2::ZERO, vec2(size.x, self.style.tab_height.min(size.y)));
                Rect::from_min_size(origin - grab, size)
            };
            ui.ctx().data_mut(|d| {
                d.insert_temp(
                    self.id.with("group-drag"),
                    GroupDrag {
                        keep_cursor: id == self.id.with(("move-group-border", panels.first())),
                        panel: first.clone(),
                        original: [rect.left(), rect.top(), rect.width(), rect.height()],
                        origin,
                        members: panels.to_vec(),
                        selected: selected.clone(),
                    },
                )
            });
        }
        if panels.iter().all(|p| (self.permissions)(p).movable)
            && response.dragged()
            && let Some(drag) = ui.ctx().data(|d| d.get_temp::<GroupDrag<P>>(self.id.with("group-drag")))
            && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
            && !ui.input(|i| i.key_pressed(Key::Escape) || !i.focused)
        {
            let [x, y, width, height] = drag.original;
            // Resolve from the original press, avoiding cumulative pixel rounding as the
            // Area follows fractional pointer steps.
            let moved = clamp_float(Rect::from_min_size(pos2(x, y), vec2(width, height)).translate(pointer - drag.origin), ui.ctx().content_rect());
            if !self.content.live_drag()
                && let Some(first) = panels.first()
            {
                self.output
                    .actions
                    .push(Action::MoveFloating { panel: first.clone(), rect: [moved.left(), moved.top(), moved.width(), moved.height()] });
            }
        }
    }

    fn group(&mut self, ui: &mut Ui, panels: &[P], active: usize, rect: Rect, floating: bool, destination: Rect) {
        let Some(selected) = panels.get(active) else { return };
        let rect = if floating { rect } else { self.presentation_rect(ui.ctx(), panels, rect) };
        self.content.group_rect(ui.ctx(), panels, rect);
        let moving = self.moving_panels(ui.ctx());
        let group_drag = ui.ctx().data(|d| d.get_temp::<GroupDrag<P>>(self.id.with("group-drag"))).is_some();
        let whole = (self.previewing || group_drag || moving.len() > 1) && !moving.is_empty() && panels.iter().all(|panel| moving.contains(panel));
        if whole {
            if !self.geometry.as_ref().is_some_and(|state| state.active) {
                ui.painter().rect_filled(rect, 0.0, self.style.background);
            }
            return;
        }

        let policy = (self.permissions)(selected);
        let bare = panels.len() == 1 && policy == Permissions::PROTECTED;
        if bare {
            self.contents(ui, selected, rect);
            self.targets.push(DropTarget {
                anchor: selected.clone(),
                rect,
                tab_strip: None,
                tabs: Vec::new(),
                allow_splits: !floating,
                accepts_tabs: false,
            });
            return;
        }
        let frozen = self.content.preview_tab_strip(ui.ctx(), panels);
        let options = super::strips::Options {
            duration: self.content.preview_geometry_duration(),
            dragging: is_dragging::<P>(ui.ctx(), self.id),
            returning: self.geometry.as_ref().is_some_and(|state| state.active),
        };
        let header = self.strips.as_mut().and_then(|state| state.header(ui.ctx(), panels, destination, frozen, options));
        if let Some(header) = header {
            let header_panels = header
                .frozen
                .as_ref()
                .map(|target| target.tabs.iter().map(|(panel, _)| panel.clone()).collect::<Vec<_>>())
                .unwrap_or_else(|| panels.to_vec());
            let header_active =
                header.frozen.as_ref().and_then(|target| header_panels.iter().position(|panel| panel == &target.anchor)).unwrap_or(active);
            self.headers.push(HeaderJob { panels: header_panels, active: header_active, rect: header.rect, floating, frozen: header.frozen });
        } else {
            self.header(ui, &HeaderJob { panels: panels.to_vec(), active, rect, floating, frozen: None });
        }
        let strip_bottom = (rect.top() + self.style.tab_height).min(rect.bottom());
        self.contents(ui, selected, Rect::from_min_max(pos2(rect.left(), strip_bottom), rect.max));
        ui.painter().rect_stroke(rect, 0.0, self.style.border, StrokeKind::Inside);
    }

    fn header(&mut self, ui: &mut Ui, job: &HeaderJob<P>) {
        let panels = &job.panels;
        let active = job.active;
        let rect = job.rect;
        let floating = job.floating;
        let Some(selected) = panels.get(active) else { return };
        let policy = (self.permissions)(selected);
        let mut strip = job
            .frozen
            .as_ref()
            .and_then(|target| target.tab_strip)
            .unwrap_or_else(|| Rect::from_min_max(rect.min, pos2(rect.right(), (rect.top() + self.style.tab_height).min(rect.bottom()))));
        ui.painter().rect_filled(strip, 0.0, self.style.background);
        ui.painter().rect_filled(strip, 0.0, self.style.tab_background);
        if floating && job.frozen.is_none() {
            let grip = Rect::from_min_max(strip.min, pos2((strip.left() + 12.0).min(strip.right()), strip.bottom()));
            self.move_floating_group(ui, panels, selected, rect, grip, self.id.with(("move-group", panels.first())));
            ui.painter().line_segment([grip.center() - vec2(0.0, 5.0), grip.center() + vec2(0.0, 5.0)], self.style.border);
            strip.min.x = grip.right();
        }
        let requested = self.content.group_header_width(panels, selected, floating);
        let header_width = if requested.is_finite() { requested.clamp(0.0, strip.width()) } else { 0.0 };
        if header_width > 0.0 {
            let header = Rect::from_min_max(pos2(strip.right() - header_width, strip.top()), strip.max);
            let mut child = ui.new_child(UiBuilder::new().id(self.id.with(("header", selected))).max_rect(header));
            child.set_clip_rect(header.intersect(ui.clip_rect()));
            self.content.group_header(&mut child, panels, selected, floating);
        }
        let widths: Vec<f32> = panels
            .iter()
            .map(|p| {
                self.content
                    .label(ui, p, &(self.label)(p), &self.style.font, self.style.text)
                    .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, self.style.font.clone())
                    .size()
                    .x
                    + 24.0
            })
            .collect();
        let fit = tabs::fit(&widths, active, (strip.width() - header_width).max(0.0), 40.0, 22.0);
        let shown = job
            .frozen
            .as_ref()
            .map(|target| {
                target
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, rect))| super::targets::usable(*rect))
                    .map(|(index, (_, rect))| (index, rect.width()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| fit.shown.clone());
        let overflow_indices = job
            .frozen
            .as_ref()
            .map(|target| {
                target.tabs.iter().enumerate().filter(|(_, (_, rect))| !super::targets::usable(*rect)).map(|(index, _)| index).collect::<Vec<_>>()
            })
            .unwrap_or_else(|| fit.overflow.clone());
        let overflow_width = if job.frozen.is_some() { 22.0_f32.min((strip.width() - header_width).max(0.0)) } else { fit.overflow_width };
        let mut x = strip.left();
        let mut targets = Vec::new();
        for (index, width) in &shown {
            let Some(panel) = panels.get(*index) else { continue };
            let tab = job
                .frozen
                .as_ref()
                .and_then(|target| target.tabs.iter().find(|(member, _)| member == panel).map(|(_, rect)| *rect))
                .unwrap_or_else(|| Rect::from_min_size(pos2(x, strip.top()), vec2(*width, strip.height())));
            self.tab(ui, panel, tab, *index == active, rect);
            targets.push((panel.clone(), tab));
            x = tab.right();
        }
        if !overflow_indices.is_empty() && overflow_width > 0.0 {
            let overflow = Rect::from_min_size(pos2(x, strip.top()), vec2(overflow_width, strip.height()));
            let response = ui.interact(overflow, self.id.with(("overflow", selected)), Sense::click());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &self.style.panels_label));
            ui.painter().with_clip_rect(overflow).text(overflow.center(), egui::Align2::CENTER_CENTER, "…", self.style.font.clone(), self.style.text);
            egui::Popup::menu(&response).show(|ui| {
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for index in &overflow_indices {
                        if let Some(panel) = panels.get(*index)
                            && self.menu_label(ui, panel).clicked()
                        {
                            self.output.actions.push(Action::Activate { panel: panel.clone() });
                            self.output.focused = Some(panel.clone());
                            ui.close();
                        }
                    }
                });
            });
        }
        if !overflow_indices.is_empty() {
            x += overflow_width;
        }
        if floating || (self.content.drag_docked_headers() && job.frozen.is_none()) {
            let blank = Rect::from_min_max(pos2(x, strip.top()), pos2((strip.right() - header_width).max(x), strip.bottom()));
            self.move_floating_group(ui, panels, selected, rect, blank, self.id.with(("move-group-header", panels.first())));
            let border = Rect::from_min_max(
                strip.min,
                pos2((strip.right() - header_width - overflow_width).max(strip.left()), (strip.top() + 4.0).min(strip.bottom())),
            );
            self.move_floating_group(ui, panels, selected, rect, border, self.id.with(("move-group-border", panels.first())));
        }
        // Retain logical order even when overflow hides a tab; hidden identities have no hit area.
        let targets = panels
            .iter()
            .map(|panel| {
                let rect = targets.iter().find(|(p, _)| p == panel).map(|(_, rect)| *rect).unwrap_or(Rect::NOTHING);
                (panel.clone(), rect)
            })
            .collect();
        self.targets.push(DropTarget {
            anchor: selected.clone(),
            rect,
            tab_strip: Some(strip),
            tabs: targets,
            allow_splits: !floating,
            accepts_tabs: policy.accepts_tabs,
        });
    }

    fn header_overlays(&mut self, ctx: &Context) {
        for job in std::mem::take(&mut self.headers) {
            let Some(anchor) = job.panels.first() else { continue };
            let id = self.surface.with(("preview-strip", anchor));
            let strip = job.frozen.as_ref().and_then(|target| target.tab_strip).unwrap_or_else(|| {
                Rect::from_min_max(job.rect.min, pos2(job.rect.right(), (job.rect.top() + self.style.tab_height).min(job.rect.bottom())))
            });
            let layer = egui::LayerId::new(egui::Order::Foreground, id);
            ctx.move_to_top(layer);
            egui::Area::new(id)
                .fade_in(false)
                .order(egui::Order::Foreground)
                .fixed_pos(strip.min)
                .movable(false)
                .interactable(false)
                .constrain(false)
                .show(ctx, |ui| {
                    ui.set_min_size(strip.size());
                    ui.set_clip_rect(strip.intersect(ctx.content_rect()));
                    ui.visuals_mut().disabled_alpha = 1.0;
                    ui.disable();
                    self.header(ui, &job);
                });
        }
    }

    fn menu_label(&mut self, ui: &mut Ui, panel: &P) -> egui::Response {
        let logical = (self.label)(panel);
        let text = self.content.label(ui, panel, &logical, &self.style.font, self.style.text);
        let response = ui.button(text);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &logical));
        response
    }

    fn tab(&mut self, ui: &mut Ui, panel: &P, rect: Rect, active: bool, group: Rect) {
        let policy = (self.permissions)(panel);
        let label = (self.label)(panel);
        let color = if active { self.style.text } else { self.style.inactive_text };
        let galley = self.content.label(ui, panel, &label, &self.style.font, color).into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            self.style.font.clone(),
        );
        let response = tabs::Tab::new(self.id.with(("tab", panel)), &label, active)
            .sense(if policy.movable { Sense::click_and_drag() } else { Sense::click() })
            .focus_stroke(Stroke::new(1.0, self.style.accent))
            .show_at(ui, rect, |ui, response| {
                let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
                painter.rect_filled(rect, 0.0, if active || response.hovered() { self.style.active_background } else { self.style.tab_background });
                painter.galley(rect.center() - galley.size() * 0.5, galley.clone(), color);
            });
        self.register_interaction(ui.ctx(), response.id);
        if response.clicked() {
            self.output.actions.push(Action::Activate { panel: panel.clone() });
            self.output.focused = Some(panel.clone());
        }
        if policy.movable && response.drag_started() {
            begin_drag(ui.ctx(), self.id, panel.clone(), group);
        }
        if self.content.live_drag() && ui.ctx().data(|d| d.get_temp::<Drag<P>>(self.id.with("drag"))).is_some_and(|d| &d.panel == panel) {
            ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())).rect_filled(rect, 0.0, self.style.tab_background);
        }
        response.context_menu(|ui| {
            self.content.panel_menu(ui, panel);
            if policy.movable && policy.floatable {
                let text = self.content.action_label(ui, &self.style.float_label, &self.style.font, self.style.text);
                let response = ui.button(text);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &self.style.float_label));
                if response.clicked() {
                    let size = self.content.floating_size(panel).filter(|s| s.is_finite() && s.x > 0.0 && s.y > 0.0).unwrap_or(group.size());
                    let rect = clamp_float(Rect::from_min_size(group.min + vec2(24.0, 24.0), size), ui.ctx().content_rect());
                    self.output.actions.push(Action::Float { panel: panel.clone(), rect: [rect.left(), rect.top(), size.x, size.y] });
                    ui.close();
                }
            }
            if policy.movable {
                let text = self.content.action_label(ui, &self.style.move_label, &self.style.font, self.style.text);
                let response = ui.menu_button(text, |ui| {
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        for target in self.layout.panels() {
                            if target != panel && (self.permissions)(target).accepts_tabs && self.menu_label(ui, target).clicked() {
                                self.output.actions.push(Action::Move {
                                    panel: panel.clone(),
                                    anchor: target.clone(),
                                    placement: Placement::Tab { before: None },
                                });
                                ui.close();
                            }
                        }
                    });
                });
                response.response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &self.style.move_label));
            }
            if policy.closable {
                let text = self.content.action_label(ui, &self.style.close_label, &self.style.font, self.style.text);
                let response = ui.button(text);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &self.style.close_label));
                if response.clicked() {
                    self.output.actions.push(Action::Close { panel: panel.clone() });
                    ui.close();
                }
            }
        });
    }

    fn floating(&mut self, ctx: &Context, display: &Layout<P>, preview: bool) {
        let screen = ctx.content_rect();
        if !super::targets::usable(screen) {
            return;
        }
        let layers = ctx.memory(|memory| memory.layer_ids().collect::<Vec<_>>());
        let mut groups: Vec<_> = display.floating.iter().collect();
        groups.sort_by_key(|group| {
            let id = group.panels.first().map(|panel| self.id.with(("floating", panel)));
            layers.iter().position(|layer| Some(layer.id) == id).unwrap_or(usize::MAX)
        });
        for group in groups {
            let Some(first) = group.panels.first() else { continue };
            let [x, y, width, height] = group.rect;
            let limits = self.node_limits(&Node::Tabs { panels: group.panels.clone(), active: group.active });
            let size = vec2(width, height).max(limits.min).min(limits.max);
            let rect = clamp_float(Rect::from_min_size(pos2(x, y), size), screen);
            let destination = rect;
            let rect = clamp_float(self.presentation_rect(ctx, &group.panels, rect), screen);
            let id = self.id.with(("floating", first));
            if self.activation.as_ref().is_some_and(|panel| group.panels.contains(panel)) {
                ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, id));
                ctx.memory_mut(|memory| memory.request_focus(self.id.with(("tab", self.activation.as_ref().unwrap_or(first)))));
                self.output.focused = self.activation.clone();
            }
            egui::Area::new(id).fade_in(false).order(egui::Order::Middle).fixed_pos(rect.min).movable(false).constrain_to(screen).show(ctx, |ui| {
                ui.set_min_size(rect.size());
                let rect = Rect::from_min_size(ui.min_rect().min, rect.size());
                ui.set_clip_rect(rect);
                if preview {
                    ui.visuals_mut().disabled_alpha = 1.0;
                    ui.disable();
                }
                ui.painter().rect_filled(rect, 0.0, self.style.background);
                let header = Rect::from_min_max(rect.min, pos2(rect.right(), (rect.top() + self.style.tab_height).min(rect.bottom())));
                ui.painter().rect_filled(header, 0.0, self.style.tab_background);
                ui.allocate_rect(rect, Sense::hover());
                self.group(ui, &group.panels, group.active, rect, true, destination);
                let corners = [
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
                        Rect::from_min_max(
                            pos2(rect.left(), (rect.bottom() - 12.0).max(rect.top())),
                            pos2((rect.left() + 12.0).min(rect.right()), rect.bottom()),
                        ),
                        vec2(-1.0, 1.0),
                    ),
                ];
                for (name, corner, direction) in corners {
                    if !super::targets::usable(corner) {
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
                    let response = ui.interact(corner, id.with(name), Sense::drag()).on_hover_cursor(cursor);
                    self.register_interaction(ui.ctx(), response.id);
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &self.style.resize_window_label));
                    let anchor_id = response.id.with("pointer-anchor");
                    if response.drag_started() {
                        response.request_focus();
                        if let Some(origin) = ui.input(|i| i.pointer.press_origin())
                            && let Some(anchor) = super::AnchoredResize::new(rect, origin, direction)
                        {
                            ui.ctx().data_mut(|data| {
                                if let Some(previous) = data.get_temp::<Id>(self.id.with("pointer-resize")) {
                                    data.remove::<super::AnchoredResize>(previous.with("pointer-anchor"));
                                }
                                data.insert_temp(self.id.with("pointer-resize"), response.id);
                                data.insert_temp(anchor_id, anchor);
                            });
                        }
                    }
                    let cancelled = ui.input(|input| input.key_pressed(Key::Escape) || !input.focused);
                    let mut delta = Vec2::ZERO;
                    if response.has_focus() {
                        ui.memory_mut(|memory| {
                            memory.set_focus_lock_filter(
                                response.id,
                                egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() },
                            )
                        });
                        ui.input_mut(|input| {
                            if input.consume_key(egui::Modifiers::NONE, Key::ArrowLeft) {
                                delta.x -= 8.0;
                            }
                            if input.consume_key(egui::Modifiers::NONE, Key::ArrowRight) {
                                delta.x += 8.0;
                            }
                            if input.consume_key(egui::Modifiers::NONE, Key::ArrowUp) {
                                delta.y -= 8.0;
                            }
                            if input.consume_key(egui::Modifiers::NONE, Key::ArrowDown) {
                                delta.y += 8.0;
                            }
                        });
                    }
                    let pointer_resize = response.dragged() || response.drag_stopped();
                    let limits = PanelLimits { min: limits.min.max(vec2(40.0, self.style.tab_height + 20.0)).min(limits.max), ..limits };
                    let next = if pointer_resize {
                        ui.ctx()
                            .data(|data| data.get_temp::<super::AnchoredResize>(anchor_id))
                            .and_then(|anchor| ui.ctx().pointer_interact_pos().and_then(|pointer| anchor.rect(pointer, screen, limits)))
                    } else if delta != Vec2::ZERO {
                        super::AnchoredResize::new(rect, rect.max, direction).and_then(|anchor| anchor.rect(rect.max + delta, screen, limits))
                    } else {
                        None
                    };
                    let permitted = response.enabled() && group.panels.iter().all(|panel| (self.permissions)(panel).movable);
                    if permitted
                        && !cancelled
                        && let Some(resized) = next.filter(|next| *next != rect)
                    {
                        self.output.actions.push(Action::MoveFloating {
                            panel: first.clone(),
                            rect: [resized.left(), resized.top(), resized.width(), resized.height()],
                        });
                    }
                    if response.drag_stopped() {
                        response.surrender_focus();
                    }
                    if cancelled || !permitted || !ui.input(|input| input.pointer.primary_down()) {
                        ui.ctx().data_mut(|data| {
                            data.remove::<super::AnchoredResize>(anchor_id);
                            if data.get_temp::<Id>(self.id.with("pointer-resize")) == Some(response.id) {
                                data.remove::<Id>(self.id.with("pointer-resize"));
                            }
                        });
                    }
                    if name == "resize" && (response.hovered() || response.dragged() || response.has_focus()) {
                        ui.painter().line_segment([corner.left_bottom(), corner.right_top()], Stroke::new(2.0, self.style.accent));
                    }
                }
            });
        }
    }

    fn prepare_preview(&mut self, ctx: &Context) -> Option<Layout<P>> {
        let active = ctx.input(|i| i.focused && !i.key_pressed(Key::Escape) && (i.pointer.primary_down() || i.pointer.primary_released()));
        let pointer = ctx.pointer_interact_pos().filter(|p| p.is_finite());
        let gesture = if active {
            if let Some(drag) = ctx.data(|d| d.get_temp::<Drag<P>>(self.id.with("drag"))) {
                pointer.map(|pointer| {
                    let floating = ((self.permissions)(&drag.panel).floatable
                        && (!drag.source.contains(pointer) || self.layout.floating.iter().any(|g| g.panels.contains(&drag.panel))))
                    .then(|| self.tab_float_rect(ctx, &drag, pointer));
                    (drag.panel, pointer, floating, false)
                })
            } else if let Some(drag) = ctx.data(|d| d.get_temp::<GroupDrag<P>>(self.id.with("group-drag"))) {
                pointer.map(|pointer| {
                    let [x, y, w, h] = drag.original;
                    let rect = clamp_float(Rect::from_min_size(pos2(x, y), vec2(w, h)).translate(pointer - drag.origin), ctx.content_rect());
                    (drag.panel, pointer, Some(rect), true)
                })
            } else {
                None
            }
        } else {
            None
        };
        let request = gesture.as_ref().filter(|(_, pointer, _, _)| !self.blockers.iter().any(|rect| rect.contains(*pointer))).map(
            |(panel, pointer, rect, whole)| DropRequest {
                panel,
                pointer: *pointer,
                targets: &self.targets,
                floating_rect: *rect,
                whole_group: *whole,
                released: false,
                workspace: self.workspace,
                screen: ctx.content_rect(),
            },
        );
        self.content.preview_layout(ctx, self.style, request.as_ref())
    }

    fn drag_ui(&mut self, ctx: &Context) {
        if !self.content.render_drag_ui() {
            return;
        }
        if !self.content.live_drag() || ctx.input(|i| !i.focused || i.key_pressed(Key::Escape) || !i.pointer.primary_down()) {
            ctx.data_mut(|d| d.remove::<DragOpacity>(self.id.with("drag-opacity")));
            return;
        }
        let Some(pointer) = ctx.pointer_interact_pos() else {
            return;
        };
        let tab = ctx.data(|d| d.get_temp::<Drag<P>>(self.id.with("drag")));
        let group = ctx.data(|d| d.get_temp::<GroupDrag<P>>(self.id.with("group-drag")));
        let (panels, selected, rect, whole_group) = if let Some(drag) = tab {
            let size = self.content.floating_size(&drag.panel).unwrap_or(drag.source.size());
            let grab = drag.grab.min(vec2(size.x, self.style.tab_height));
            (
                self.content.drag_members(ctx, &drag.panel).unwrap_or_else(|| vec![drag.panel.clone()]),
                drag.panel,
                clamp_float(Rect::from_min_size(pointer - grab, size), ctx.content_rect()),
                false,
            )
        } else if let Some(drag) = group {
            let [x, y, w, h] = drag.original;
            (
                drag.members.clone(),
                drag.selected.clone(),
                clamp_float(Rect::from_min_size(pos2(x, y), vec2(w, h)).translate(pointer - drag.origin), ctx.content_rect()),
                true,
            )
        } else {
            ctx.data_mut(|d| d.remove::<DragOpacity>(self.id.with("drag-opacity")));
            return;
        };
        let opacity = self.content.drag_opacity(
            ctx,
            &DropRequest {
                panel: &selected,
                pointer,
                targets: &self.targets,
                floating_rect: Some(rect),
                whole_group,
                released: false,
                workspace: self.workspace,
                screen: ctx.content_rect(),
            },
        );
        let opacity = if opacity.is_finite() { opacity.clamp(0.0, 1.0) } else { 1.0 };
        let duration = self.content.drag_opacity_duration();
        let now = ctx.input(|i| i.time);
        let opacity = if duration.is_finite() && duration > 0.0 {
            let (current, repaint) = ctx.data_mut(|d| {
                let key = self.id.with("drag-opacity");
                let mut state = d.get_temp::<DragOpacity>(key).unwrap_or(DragOpacity { from: 1.0, target: 1.0, started: now, current: 1.0 });
                let progress = ((now - state.started) / f64::from(duration)).clamp(0.0, 1.0) as f32;
                let eased = progress * progress * (3.0 - 2.0 * progress);
                state.current = state.from + (state.target - state.from) * eased;
                if state.target != opacity {
                    state.from = state.current;
                    state.target = opacity;
                    state.started = now;
                }
                let repaint = state.current != state.target;
                let current = state.current;
                d.insert_temp(key, state);
                (current, repaint)
            });
            if repaint {
                ctx.request_repaint();
            }
            current
        } else {
            opacity
        };
        egui::Area::new(self.id.with("drag-ui"))
            .fade_in(false)
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .movable(false)
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_min_size(rect.size());
                ui.visuals_mut().disabled_alpha = 1.0;
                ui.disable();
                ui.set_opacity(opacity);
                ui.set_clip_rect(rect);
                ui.painter().rect_filled(rect, 0.0, self.style.background);
                let header = Rect::from_min_size(rect.min, vec2(rect.width(), self.style.tab_height.min(rect.height())));
                ui.painter().rect_filled(header, 0.0, self.style.tab_background);
                let mut x = header.left();
                for panel in panels {
                    let text = (self.label)(&panel);
                    let galley = ui.painter().layout_no_wrap(text, self.style.font.clone(), self.style.text);
                    let width = (galley.size().x + 20.0).min((header.right() - x).max(0.0));
                    let tab = Rect::from_min_size(pos2(x, header.top()), vec2(width, header.height()));
                    if panel == selected {
                        ui.painter().rect_filled(tab, 0.0, self.style.active_background);
                    }
                    ui.painter().with_clip_rect(tab).galley(pos2(x + 10.0, header.center().y - galley.size().y / 2.0), galley, self.style.text);
                    x += width;
                }
                let body = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max);
                let mut child = ui.new_child(UiBuilder::new().id(self.id.with(("body", &selected))).max_rect(body));
                child.set_clip_rect(body);
                self.content.body(&mut child, &selected);
                ui.painter().rect_stroke(rect, 0.0, self.style.border, StrokeKind::Inside);
            });
    }

    fn host_drop(&mut self, ctx: &Context, panel: &P, pointer: Pos2, whole_group: bool, released: bool, floating_rect: Option<Rect>) -> bool {
        let request = DropRequest {
            panel,
            pointer,
            targets: &self.targets,
            floating_rect,
            whole_group,
            released,
            workspace: self.workspace,
            screen: ctx.content_rect(),
        };
        let Some(feedback) = self.content.resolve_drag(ctx, self.style, &request) else { return false };
        if !released {
            let layer = egui::LayerId::new(egui::Order::Tooltip, self.id.with("drop-preview"));
            ctx.move_to_top(layer);
            let painter = ctx.layer_painter(layer);
            if let Some(rect) = feedback.preview.filter(|rect| super::targets::usable(*rect)) {
                painter.rect(rect, 0.0, self.style.accent.gamma_multiply(0.15), Stroke::new(2.0, self.style.accent), StrokeKind::Inside);
            }
            if let Some(rect) = feedback.insertion.filter(|rect| super::targets::usable(*rect)) {
                painter.rect_filled(rect, 0.0, self.style.accent);
            }
        }
        true
    }

    fn finish_group_drag(&mut self, ctx: &Context) {
        let Some(drag) = ctx.data(|d| d.get_temp::<GroupDrag<P>>(self.id.with("group-drag"))) else { return };
        if !drag.members.iter().all(|panel| self.layout.contains(panel) && (self.permissions)(panel).movable) {
            if !self.content.live_drag() {
                ctx.data_mut(|d| d.remove::<GroupDrag<P>>(self.id.with("group-drag")));
            }
            return;
        }
        if drag.keep_cursor {
            ctx.set_cursor_icon(egui::CursorIcon::Default);
        }
        let (escape, focused, down, released, pointer) =
            ctx.input(|i| (i.key_pressed(Key::Escape), i.focused, i.pointer.primary_down(), i.pointer.primary_released(), i.pointer.interact_pos()));
        if !escape
            && focused
            && (down || released)
            && let Some(pointer) = pointer.filter(|p| p.is_finite())
            && !self.blockers.iter().any(|r| r.contains(pointer))
            && self.host_drop(
                ctx,
                &drag.panel,
                pointer,
                true,
                released,
                self.content.live_drag().then(|| {
                    let [x, y, w, h] = drag.original;
                    clamp_float(Rect::from_min_size(pos2(x, y), vec2(w, h)).translate(pointer - drag.origin), ctx.content_rect())
                }),
            )
        {
            if released {
                ctx.data_mut(|d| d.remove::<GroupDrag<P>>(self.id.with("group-drag")));
            }
            return;
        }
        if escape || !focused || !down {
            ctx.data_mut(|d| d.remove::<GroupDrag<P>>(self.id.with("group-drag")));
            if escape || !focused {
                if !self.content.live_drag() {
                    self.output.actions.push(Action::MoveFloating { panel: drag.panel, rect: drag.original });
                }
            } else if released
                && let Some(pointer) = pointer.filter(|p| p.is_finite())
                && !self.blockers.iter().any(|r| r.contains(pointer))
            {
                self.output.group_drop = Some((drag.panel, pointer));
            }
        } else if pointer.is_some_and(|p| !self.blockers.iter().any(|r| r.contains(p))) {
            self.output.group_drag = Some(drag.panel);
        }
    }

    fn tab_float_rect(&self, ctx: &Context, drag: &Drag<P>, pointer: Pos2) -> Rect {
        let size = self.content.floating_size(&drag.panel).filter(|s| s.is_finite() && s.x > 0.0 && s.y > 0.0).unwrap_or(drag.source.size());
        let grab = drag.grab.min(vec2(size.x, self.style.tab_height));
        clamp_float(Rect::from_min_size(pointer - grab, size), ctx.content_rect())
    }

    fn finish_drag(&mut self, ctx: &Context) {
        let Some(drag) = ctx.data(|d| d.get_temp::<Drag<P>>(self.id.with("drag"))) else { return };
        let (pointer, down, released, focused) =
            ctx.input(|i| (i.pointer.interact_pos(), i.pointer.primary_down(), i.pointer.primary_released(), i.focused));
        let escape = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
        if escape || !focused || (!self.content.live_drag() && !self.layout.contains(&drag.panel)) || (!down && !released) {
            cancel_drag::<P>(ctx, self.id);
            return;
        }
        let Some(pointer) = pointer.filter(|p| p.is_finite()) else {
            if released {
                cancel_drag::<P>(ctx, self.id);
            }
            return;
        };
        if self.blockers.iter().any(|rect| rect.contains(pointer)) {
            if released {
                cancel_drag::<P>(ctx, self.id);
            }
            return;
        }
        let floating_rect = ((self.permissions)(&drag.panel).floatable
            && (!drag.source.contains(pointer) || self.layout.floating.iter().any(|g| g.panels.contains(&drag.panel))))
        .then(|| self.tab_float_rect(ctx, &drag, pointer));
        if self.host_drop(ctx, &drag.panel, pointer, false, released, floating_rect) {
            if released {
                cancel_drag::<P>(ctx, self.id);
            }
            return;
        }
        let proposal = resolve_drop(&drag.panel, pointer, &self.targets);
        if released && proposal.as_ref().is_some_and(|proposal| proposal.noop) {
            if released {
                cancel_drag::<P>(ctx, self.id);
            }
            return;
        }
        if released {
            cancel_drag::<P>(ctx, self.id);
            if pointer.distance(drag.origin) < 2.0 {
                return;
            }
            let action = if let Some(proposal) = proposal {
                Some(Action::Move { panel: drag.panel.clone(), anchor: proposal.anchor, placement: proposal.placement })
            } else if (self.permissions)(&drag.panel).floatable
                && (!drag.source.contains(pointer) || self.layout.floating.iter().any(|g| g.panels.contains(&drag.panel)))
            {
                let rect = self.tab_float_rect(ctx, &drag, pointer);
                Some(Action::Float { panel: drag.panel.clone(), rect: [rect.left(), rect.top(), rect.width(), rect.height()] })
            } else {
                None
            };
            if let Some(action) = action {
                // Validate the proposed gesture against the same complete snapshot. The app
                // still applies it via its own command path after body rendering finishes.
                let mut candidate = self.layout.clone();
                if candidate.apply_with_permissions(action.clone(), self.permissions).is_ok() {
                    self.output.actions.push(action);
                    self.output.focused = Some(drag.panel);
                }
            }
            return;
        }
        let layer = egui::LayerId::new(egui::Order::Tooltip, self.id.with("drop-preview"));
        ctx.move_to_top(layer);
        let painter = ctx.layer_painter(layer);
        if let Some(proposal) = proposal {
            let same_group = proposal.insertion.is_some()
                && self.targets.iter().any(|target| target.rect == proposal.preview && target.tabs.iter().any(|(panel, _)| panel == &drag.panel));
            if !same_group {
                painter.rect(
                    proposal.preview.shrink(1.0),
                    0.0,
                    self.style.accent.gamma_multiply(0.15),
                    Stroke::new(2.0, self.style.accent),
                    StrokeKind::Inside,
                );
            }
            if let Some(marker) = proposal.insertion {
                painter.rect_filled(marker, 0.0, self.style.accent);
            }
        } else if (self.permissions)(&drag.panel).floatable {
            let rect = self.tab_float_rect(ctx, &drag, pointer);
            painter.rect_stroke(rect, 0.0, Stroke::new(2.0, self.style.accent), StrokeKind::Inside);
        }
    }
}

fn clamp_float(rect: Rect, screen: Rect) -> Rect {
    if !super::targets::usable(screen) {
        return rect;
    }
    let size = rect.size().max(vec2(1.0, 1.0)).min(screen.size());
    let min = rect.min.max(screen.min).min(screen.max - size);
    Rect::from_min_size(min, size)
}

fn split_bounds(available: f32, axis: SplitAxis, first: PanelLimits, second: PanelLimits) -> (f32, f32) {
    let (a_min, a_max, b_min, b_max) = match axis {
        SplitAxis::Horizontal => (first.min.x, first.max.x, second.min.x, second.max.x),
        SplitAxis::Vertical => (first.min.y, first.max.y, second.min.y, second.max.y),
    };
    let scale = if a_min + b_min > available { available / (a_min + b_min).max(1.0) } else { 1.0 };
    let lo = (a_min * scale).min(available);
    let hi = (available - b_min * scale).max(lo).min(available);
    let constrained_lo = lo.max(available - b_max);
    let constrained_hi = hi.min(a_max);
    if constrained_lo <= constrained_hi { (constrained_lo, constrained_hi) } else { (lo, hi) }
}

pub(super) fn node_limits<P>(
    node: &Node<P>,
    style: &DockStyle,
    permissions: &impl Fn(&P) -> Permissions,
    limits_fn: &impl Fn(&P) -> PanelLimits,
) -> PanelLimits {
    match node {
        Node::Tabs { panels, .. } => {
            let mut limits = PanelLimits::default();
            for panel in panels {
                let policy = (limits_fn)(panel);
                limits.min = limits.min.max(policy.min).max(Vec2::splat(style.min_pane));
                limits.max = limits.max.min(policy.max);
            }
            limits.max = limits.max.max(limits.min);
            if panels.len() != 1 || panels.first().is_some_and(|p| (permissions)(p) != Permissions::PROTECTED) {
                limits.min.y = (limits.min.y + style.tab_height).min(1_000_000.0);
                limits.max.y = (limits.max.y + style.tab_height).min(1_000_000.0);
            }
            limits
        }
        Node::Stack { entries } => {
            let mut limits = PanelLimits::default();
            for entry in entries {
                let policy = (limits_fn)(&entry.panel);
                limits.min.x = limits.min.x.max(policy.min.x).max(style.min_pane);
                limits.min.y += if entry.open { policy.min.y.max(style.min_pane) } else { 0.0 } + style.tab_height;
                limits.max.x = limits.max.x.min(policy.max.x);
            }
            limits.min = limits.min.min(Vec2::splat(1_000_000.0));
            limits.max = limits.max.max(limits.min);
            limits
        }
        Node::Split { axis, first, second, .. } => {
            let a = node_limits(first, style, permissions, limits_fn);
            let b = node_limits(second, style, permissions, limits_fn);
            let limits = match axis {
                SplitAxis::Horizontal => PanelLimits {
                    min: vec2(a.min.x + b.min.x + style.gap, a.min.y.max(b.min.y)),
                    max: vec2(a.max.x + b.max.x + style.gap, a.max.y.min(b.max.y)),
                },
                SplitAxis::Vertical => PanelLimits {
                    min: vec2(a.min.x.max(b.min.x), a.min.y + b.min.y + style.gap),
                    max: vec2(a.max.x.min(b.max.x), a.max.y + b.max.y + style.gap),
                },
            };
            PanelLimits { min: limits.min.min(Vec2::splat(1_000_000.0)), max: limits.max.max(limits.min).min(Vec2::splat(1_000_000.0)) }
        }
    }
}

/// Measure full group rectangles with exactly the constraints used by DockArea. Validates
/// the tree first, bounding recursion and rejecting hostile geometry before traversing it.
pub fn measure_layout<P: Clone + Eq + Hash>(
    layout: &Layout<P>,
    workspace: Rect,
    screen: Rect,
    style: &DockStyle,
    permissions: impl Fn(&P) -> Permissions,
    limits: impl Fn(&P) -> PanelLimits,
) -> Result<Vec<(P, Rect)>, DockError> {
    measure_layout_with_boundaries(layout, workspace, screen, style, permissions, limits).map(|measured| measured.panels)
}

/// Authoritative group bounds and real internal dividers, sharing the renderer's constraints.
#[derive(Clone, Debug)]
pub struct LayoutMeasurement<P> {
    pub panels: Vec<(P, Rect)>,
    pub boundaries: Vec<SplitBoundary<P>>,
}

#[derive(Clone, Debug)]
pub struct SplitBoundary<P> {
    pub axis: SplitAxis,
    pub rect: Rect,
    pub divider: Rect,
    pub first_rect: Rect,
    pub second_rect: Rect,
    pub first: Vec<P>,
    pub second: Vec<P>,
}

pub fn measure_layout_with_boundaries<P: Clone + Eq + Hash>(
    layout: &Layout<P>,
    workspace: Rect,
    screen: Rect,
    style: &DockStyle,
    permissions: impl Fn(&P) -> Permissions,
    limits: impl Fn(&P) -> PanelLimits,
) -> Result<LayoutMeasurement<P>, DockError> {
    layout.validate()?;
    if !style.valid() || !super::targets::usable(workspace) || !super::targets::usable(screen) || layout.panels().iter().any(|p| !limits(p).valid()) {
        return Err(DockError::InvalidGeometry);
    }
    let mut output = Vec::new();
    let mut boundaries = Vec::new();
    if let Some(root) = &layout.root {
        measure_node(root, workspace, style, &permissions, &limits, &mut output, &mut boundaries);
    }
    for group in &layout.floating {
        let policy = node_limits(&Node::Tabs { panels: group.panels.clone(), active: group.active }, style, &permissions, &limits);
        let [x, y, w, h] = group.rect;
        let rect = clamp_float(Rect::from_min_size(pos2(x, y), vec2(w, h).max(policy.min).min(policy.max)), screen);
        output.extend(group.panels.iter().cloned().map(|p| (p, rect)));
    }
    Ok(LayoutMeasurement { panels: output, boundaries })
}

fn measure_node<P: Clone>(
    node: &Node<P>,
    rect: Rect,
    style: &DockStyle,
    permissions: &impl Fn(&P) -> Permissions,
    limits: &impl Fn(&P) -> PanelLimits,
    output: &mut Vec<(P, Rect)>,
    boundaries: &mut Vec<SplitBoundary<P>>,
) {
    match node {
        Node::Split { axis, size, first, second } => {
            let Some((split, _, _)) = constrained_split(
                rect,
                *axis,
                *size,
                style.gap,
                node_limits(first, style, permissions, limits),
                node_limits(second, style, permissions, limits),
            ) else {
                return;
            };
            let a = output.len();
            measure_node(first, split.first, style, permissions, limits, output, boundaries);
            let b = output.len();
            measure_node(second, split.second, style, permissions, limits, output, boundaries);
            boundaries.push(SplitBoundary {
                axis: *axis,
                rect,
                divider: split.divider,
                first_rect: split.first,
                second_rect: split.second,
                first: output.get(a..b).unwrap_or_default().iter().map(|(panel, _)| panel.clone()).collect(),
                second: output.get(b..).unwrap_or_default().iter().map(|(panel, _)| panel.clone()).collect(),
            });
        }
        Node::Tabs { panels, .. } => output.extend(panels.iter().cloned().map(|p| (p, rect))),
        Node::Stack { entries } => {
            let header = style.tab_height.min(rect.height() / entries.len().max(1) as f32);
            let remaining = (rect.height() - (entries.len() as f32 * style.tab_height).min(rect.height())).max(0.0);
            let total: f64 = entries.iter().filter(|e| e.open).map(|e| f64::from(e.height.unwrap_or(style.min_pane).max(1.0))).sum();
            let mut top = rect.top();
            for entry in entries {
                let share = if entry.open && total > 0.0 {
                    (f64::from(remaining) * f64::from(entry.height.unwrap_or(style.min_pane).max(1.0)) / total) as f32
                } else {
                    0.0
                };
                let pane = Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), (top + header + share).min(rect.bottom())));
                output.push((entry.panel.clone(), pane));
                top = pane.bottom();
            }
        }
    }
}

fn constrained_split(
    rect: Rect,
    axis: SplitAxis,
    size: SplitSize,
    gap: f32,
    first: PanelLimits,
    second: PanelLimits,
) -> Option<(crate::layout::SplitLayout, f32, f32)> {
    let wanted = split_rect(rect, axis, size, gap, 0.0)?;
    let (lo, hi) = split_bounds(wanted.available(), axis, first, second);
    let split = split_rect(rect, axis, SplitSize::FixedFirst(wanted.first_size().clamp(lo, hi)), gap, 0.0)?;
    Some((split, lo, hi))
}
