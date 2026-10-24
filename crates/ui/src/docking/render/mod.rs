use std::{collections::HashMap, fmt::Debug, hash::Hash};

use egui::{Context, CursorIcon, Id, Rect, Sense, Ui, UiBuilder, Vec2, pos2, vec2};

use crate::layout::{ResizeHandle, ResizeHandleStyle, SplitAxis};

use super::{Action, DockContent, DockOutput, DockStyle, DropTarget, Layout, Node, PanelLimits, Permissions, dock_model::Place};

#[cfg(feature = "native-windows")]
mod cross;
mod dock;
mod floating;
mod frame;
mod iconic;
#[cfg(feature = "native-windows")]
mod native;
#[cfg(all(test, feature = "native-windows"))]
pub(in crate::docking) use {
    cross::DragPointer,
    native::{APPKIT_CORNER, MACOS_CLOSE, Session, WindowDrag, appkit_resize_areas, close_rects, floats_shown, resize_bands, session, wayland},
};

/// The viewport of the native window `window` of the dock area `area`.
#[cfg(feature = "native-windows")]
pub(in crate::docking) fn native_viewport(area: Id, window: u64) -> egui::ViewportId {
    native::viewport_id(area, window, 0)
}

pub(in crate::docking) use frame::{FrameState, view};

pub(in crate::docking) use group::GroupKind;

pub(in crate::docking) use iconic::Iconic;
pub use iconic::{close_flyout, flyout, open_flyout};

#[cfg(test)]
pub(in crate::docking) use dock::{GroupSize, fit_widths, group_heights};
pub(in crate::docking) use floating::RESIZE_HANDLES;
mod group;
mod preview;
mod sizes;

pub(in crate::docking) use sizes::{DROP_MARGIN, Drawn, RowItem, Sizer, row_drag, row_range};

pub(super) struct Renderer<'a, P> {
    pub(super) id: Id,
    pub(super) style: &'a DockStyle,
    pub(super) content: &'a mut dyn DockContent<P>,
    pub(super) activation: Option<P>,
    pub(super) blockers: Vec<Rect>,
    /// [`crate::docking::DockArea::locked`].
    pub(super) locked: bool,
    pub(super) layout: &'a Layout<P>,
    /// The docked area, for host drop requests.
    pub(super) workspace: Rect,
    /// Where floating windows, flyouts and drag ghosts go: [`crate::docking::DockArea::float_bounds`].
    pub(super) float_bounds: Rect,
    pub(super) targets: Vec<DropTarget<P>>,
    /// [D] Seams, group bands, column edges and new-dock bands, see [`crate::docking::targets::DockDrop`].
    pub(super) dock_targets: Vec<crate::docking::targets::DockDrop<P>>,
    pub(super) owned_interactions: Vec<Id>,
    pub(super) output: DockOutput<P>,
    pub(super) frame: FrameState<P>,
    /// [D] Flyout and iconic column state for this frame.
    pub(super) iconic: Iconic<P>,
    /// What a drag would float from each drawn panel, group and column, for commands.
    pub(super) float_space: crate::docking::FloatSpace<P>,
    /// Each panel's place in `layout`, and its entry in `float_space.contents`.
    pub(super) places: HashMap<P, Place>,
    pub(super) contents: HashMap<P, usize>,
    /// Split dividers drawn so far, by view path, for the intersections of their ancestors.
    pub(super) dividers: Vec<DrawnDivider>,
    /// The floating window whose tree is being drawn: its first panel, which names it in
    /// ids and actions, and its rect. `None` while the docked layout draws.
    pub(super) tree: Option<(P, Rect)>,
    /// [`crate::docking::DockArea::monitors`].
    #[cfg_attr(not(feature = "native-windows"), allow(dead_code))]
    pub(super) monitors: Vec<crate::docking::Monitor>,
    /// [`crate::docking::DockArea::native_windows`].
    #[cfg_attr(not(feature = "native-windows"), allow(dead_code))]
    pub(super) native_windows: Option<bool>,
    /// The other OS windows a drag may cross into.
    #[cfg(feature = "native-windows")]
    pub(super) cross: cross::Cross<P>,
}

pub(super) struct DrawnDivider {
    path: Vec<bool>,
    axis: SplitAxis,
    divider: Rect,
}

impl<P: Clone + Eq + Hash + Debug + Send + Sync + 'static> Renderer<'_, P> {
    pub(super) fn place(&self, panel: &P) -> Option<&Place> {
        self.places.get(panel)
    }

    /// The height `panel`'s content last used: `Some(None)` when it filled its body.
    pub(super) fn content_height(&self, panel: &P) -> Option<Option<f32>> {
        self.contents.get(panel).and_then(|i| self.float_space.contents.get(*i)).map(|(_, height)| *height)
    }

    pub(super) fn label_of(&self, panel: &P) -> String {
        self.content.logical_label(panel)
    }

    pub(super) fn permissions(&self, panel: &P) -> Permissions {
        let permissions = self.content.permissions(panel);
        if self.locked { permissions.locked() } else { permissions }
    }

    pub(super) fn limits(&self, panel: &P) -> PanelLimits {
        self.content.limits(panel)
    }

    pub(super) fn register_interaction(&mut self, ctx: &Context, id: Id) {
        // A validated tree has at most 2*MAX_PANELS nodes. Tabs, stack dividers,
        // floating grips/corners and splits fit comfortably inside this fixed bound.
        const CAP: usize = 8 * crate::docking::MAX_PANELS;
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

    /// Draws `node` at view path `path` in `rect`, in the tree `root` (its root node and rect),
    /// which a dragged divider resizes beyond `node`.
    pub(in crate::docking) fn node(&mut self, ui: &mut Ui, node: &Node<P>, rect: Rect, path: &mut Vec<bool>, root: (&Node<P>, Rect)) {
        if !crate::docking::targets::usable(rect) {
            return;
        }
        self.note_edge(path, rect);
        match node {
            Node::Split { axis: SplitAxis::Horizontal, first, second, .. } if self.dock_split(ui, first, second, rect, path, root) => {}
            Node::Split { axis, size, first, second } => {
                let sizer = self.sizer();
                let Some(split) = sizer.split(*axis, *size, first, second, rect) else { return };
                let (lo, hi) = sizer.range(root, path);
                let mark = self.dividers.len();
                path.push(false);
                self.node(ui, first, split.first, path, root);
                path.pop();
                path.push(true);
                self.node(ui, second, split.second, path, root);
                path.pop();
                // The divider's inner-edge hit margin must win over child scrollbars, just
                // as accordion dividers win over the adjacent panel's content.
                // Named by its layout path, which `ResizeSplit` takes and hits report.
                let layout_path = self.original_path(path);
                let response = ResizeHandle::new(
                    self.split_id(&layout_path),
                    *axis,
                    split.divider,
                    &self.style.resize_label,
                    ResizeHandleStyle { hovered: self.style.accent, dragged: self.style.accent, focused: self.style.accent },
                )
                .value(split.first_size(), (split.first_size() + lo)..=(split.first_size() + hi))
                .show(ui);
                self.register_interaction(ui.ctx(), response.response.id);
                if response.delta != 0.0 {
                    let actions = self.divider_actions(root, path, response.delta);
                    self.output.actions.extend(actions);
                }
                self.intersections(ui, (node, rect), path, split.divider, mark, root);
                if self.dividers.len() < crate::docking::model::MAX_NODES {
                    self.dividers.push(DrawnDivider { path: path.clone(), axis: *axis, divider: split.divider });
                }
            }
            Node::Tabs { panels, active } => {
                let kind = match &self.tree {
                    None => GroupKind::Frame,
                    Some((_, window)) => GroupKind::Floating { window: *window },
                };
                self.group(ui, panels, *active, false, rect, kind);
            }
            Node::Dock { side, columns } => self.dock(ui, *side, columns, rect, rect.width()),
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
                    let title_rect = Rect::from_min_max(panel_rect.min, pos2(panel_rect.right(), (top + header).min(panel_rect.bottom())));
                    self.tab(ui, &entry.panel, title_rect, entry.open, panel_rect, None);
                    let collapse = if self.style.right_to_left {
                        Rect::from_min_max(title_rect.min, pos2((title_rect.left() + 18.0).min(title_rect.right()), title_rect.bottom()))
                    } else {
                        Rect::from_min_max(pos2((title_rect.right() - 18.0).max(title_rect.left()), title_rect.top()), title_rect.max)
                    };
                    let response = ui.interact(collapse, self.id.with(("collapse", &entry.panel)), Sense::click());
                    self.register_interaction(ui.ctx(), response.id);
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), entry.open, self.label_of(&entry.panel))
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
                    let body_rect = Rect::from_min_max(pos2(rect.left(), title_rect.bottom()), panel_rect.max);
                    if entry.open {
                        self.contents(ui, &entry.panel, body_rect);
                    }
                    self.targets.push(DropTarget {
                        anchor: entry.panel.clone(),
                        rect: panel_rect,
                        tab_strip: Some(title_rect),
                        tabs: vec![(entry.panel.clone(), title_rect)],
                        allow_splits: self.permissions(&entry.panel).accepts_splits,
                        accepts_tabs: self.permissions(&entry.panel).accepts_tabs,
                        right_to_left: self.style.right_to_left,
                    });
                    top = panel_rect.bottom();
                    if entry.open && remaining > 0.0 {
                        dividers.push((index, Rect::from_min_max(pos2(rect.left(), top - 1.0), pos2(rect.right(), top + 1.0)).intersect(rect)));
                    }
                }
                // Register dividers after the headers so their narrow hit regions win at the
                // shared boundary. Closed entries retain their saved sizes and are skipped.
                let scale = if total_wanted > 0.0 { f64::from(remaining) / total_wanted } else { 0.0 };
                let body_limits = |panel: &P| {
                    let mut limits = self.limits(panel);
                    limits.min.y = limits.min.y.max(self.style.min_pane);
                    limits.max = limits.max.max(limits.min);
                    limits
                };
                let open: Vec<&crate::docking::StackEntry<P>> = entries.iter().filter(|entry| entry.open).collect();
                let row: Vec<RowItem> = open
                    .iter()
                    .map(|entry| {
                        let limits = body_limits(&entry.panel);
                        let size = (f64::from(entry.height.unwrap_or(self.style.min_pane).max(1.0)) * scale) as f32;
                        RowItem { size, min: limits.min.y, max: limits.max.y }
                    })
                    .collect();
                for (index, divider) in dividers {
                    let Some(first) = entries.get(index) else { continue };
                    let Some(position) = open.iter().position(|entry| std::ptr::eq(*entry, first)) else { continue };
                    let Some(first_size) = row.get(position).map(|item| item.size) else { continue };
                    if position + 1 >= row.len() {
                        continue;
                    }
                    let (lo, hi) = row_range(&row, position);
                    let response = ResizeHandle::new(
                        self.id.with(("stack-resize", &first.panel)),
                        SplitAxis::Vertical,
                        divider,
                        &self.style.resize_label,
                        ResizeHandleStyle { hovered: self.style.accent, dragged: self.style.accent, focused: self.style.accent },
                    )
                    .value(first_size, (first_size + lo)..=(first_size + hi))
                    .show(ui);
                    self.register_interaction(ui.ctx(), response.response.id);
                    if response.delta != 0.0 && scale > 0.0 {
                        for ((entry, item), next) in open.iter().zip(&row).zip(row_drag(&row, position, response.delta)) {
                            if next != item.size {
                                let height = (f64::from(next) / scale).clamp(1.0, 1_000_000.0) as f32;
                                self.output.actions.push(Action::ResizeStack { panel: entry.panel.clone(), height: Some(height) });
                            }
                        }
                    }
                }
            }
        }
    }

    pub(super) fn sizer(&self) -> Sizer<'_, P> {
        Sizer { style: self.style, content: &*self.content }
    }

    /// `ResizeSplit` actions that move the divider of the split at view path `path` in the
    /// tree `root` by `delta` points.
    fn divider_actions(&self, root: (&Node<P>, Rect), path: &[bool], delta: f32) -> Vec<Action<P>> {
        let sizes = self.sizer().drag(root, path, delta);
        let resize = |path: Vec<bool>, size| match &self.tree {
            None => Action::ResizeSplit { path, size },
            Some((panel, _)) => Action::ResizeWindowSplit { panel: panel.clone(), path, size },
        };
        sizes.into_iter().map(|(path, size)| resize(self.original_path(&path), size)).collect()
    }

    /// The id of the divider of the split at `path` in the tree being drawn, as
    /// [`crate::docking::DockArea::hit_id`] names it.
    fn split_id(&self, path: &[bool]) -> Id {
        match &self.tree {
            None => self.id.with(("split", path)),
            Some((panel, _)) => self.id.with(("window-split", panel, path)),
        }
    }

    /// Four-way handles where a divider drawn inside the split `node` (since `mark`) ends on
    /// the split's own divider: dragging one moves both dividers.
    fn intersections(&mut self, ui: &mut Ui, (node, rect): (&Node<P>, Rect), path: &[bool], divider: Rect, mark: usize, root: (&Node<P>, Rect)) {
        let Node::Split { axis, .. } = node else { return };
        let side = (self.style.gap * 3.0).max(12.0);
        let mut found = Vec::new();
        for inner in self.dividers.get(mark..).unwrap_or_default() {
            if inner.axis == *axis || !inner.divider.expand(1.0).intersects(divider) {
                continue;
            }
            let center = match axis {
                SplitAxis::Horizontal => pos2(divider.center().x, inner.divider.center().y),
                SplitAxis::Vertical => pos2(inner.divider.center().x, divider.center().y),
            };
            found.push((inner.path.clone(), Rect::from_center_size(center, Vec2::splat(side)).intersect(rect)));
        }
        for (inner_path, hit) in found {
            let Some(inner) = inner_path.get(path.len()..).and_then(|rest| crate::docking::dock_model::node_at(node, rest)) else { continue };
            let (outer, inner_layout) = (self.original_path(path), self.original_path(&inner_path));
            let id = match &self.tree {
                None => self.id.with(("intersection", outer, inner_layout)),
                Some((panel, _)) => self.id.with(("window-intersection", panel, outer, inner_layout)),
            };
            // Pointer only: each of the two dividers it joins already takes keyboard focus.
            let response = ui.interact(hit, id, Sense::DRAG);
            self.register_interaction(ui.ctx(), id);
            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(CursorIcon::Move);
            }
            let delta = response.drag_delta();
            if delta != Vec2::ZERO && delta.is_finite() {
                let along = |axis: SplitAxis| if axis == SplitAxis::Horizontal { delta.x } else { delta.y };
                let Node::Split { axis: inner_axis, .. } = inner else { continue };
                let mut actions = self.divider_actions(root, path, along(*axis));
                actions.extend(self.divider_actions(root, &inner_path, along(*inner_axis)));
                self.output.actions.extend(actions);
            }
        }
    }

    pub(super) fn contents(&mut self, ui: &mut Ui, panel: &P, rect: Rect) {
        if !crate::docking::targets::usable(rect) {
            return;
        }
        ui.painter().rect_filled(rect, 0.0, self.style.background);
        let mut child =
            ui.new_child(UiBuilder::new().id(self.id.with(("body", panel))).max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        self.content.body(&mut child, panel);
        let at = self.contents.get(panel).copied();
        let at = self.float_space.record_content(at, panel.clone(), child.min_rect().height(), rect.height());
        self.contents.insert(panel.clone(), at);
        // Any button: a canvas pans with the middle button and opens menus with the secondary.
        if child.rect_contains_pointer(rect) && child.input(|i| i.pointer.any_pressed()) {
            self.output.focused = Some(panel.clone());
        }
    }
}

/// A lone protected panel draws no tab strip and no drag bar.
pub(in crate::docking) fn stripless<P>(panels: &[P], content: &dyn DockContent<P>) -> bool {
    match panels {
        [panel] => content.permissions(panel).is_protected(),
        _ => false,
    }
}

/// Size policy of a subtree, shared by the renderer and its measurements.
pub(in crate::docking) fn node_limits<P>(node: &Node<P>, style: &DockStyle, content: &dyn DockContent<P>) -> PanelLimits {
    match node {
        Node::Dock { columns, .. } => {
            let width: f32 = columns.iter().map(|c| dock::column_width(c, style)).sum();
            PanelLimits { min: vec2(width.clamp(style.min_pane, 1_000_000.0), style.min_pane), ..PanelLimits::default() }
        }
        // A group is as large as its largest panel allows: a panel without a maximum lifts the
        // maximum of the panels it joins.
        Node::Tabs { panels, .. } => {
            let mut limits = PanelLimits { min: Vec2::ZERO, max: if panels.is_empty() { PanelLimits::default().max } else { Vec2::ZERO } };
            for panel in panels {
                let policy = content.limits(panel);
                limits.min = limits.min.max(policy.min).max(Vec2::splat(style.min_pane));
                limits.max = limits.max.max(policy.max);
            }
            limits.max = limits.max.max(limits.min);
            if !stripless(panels, content) {
                limits.min.y = (limits.min.y + style.tab_height).min(1_000_000.0);
                limits.max.y = (limits.max.y + style.tab_height).min(1_000_000.0);
            }
            limits
        }
        Node::Stack { entries } => {
            let mut limits = PanelLimits::default();
            if !entries.is_empty() {
                limits.max.x = 0.0;
            }
            for entry in entries {
                let policy = content.limits(&entry.panel);
                limits.min.x = limits.min.x.max(policy.min.x).max(style.min_pane);
                limits.min.y += if entry.open { policy.min.y.max(style.min_pane) } else { 0.0 } + style.tab_height;
                limits.max.x = limits.max.x.max(policy.max.x);
            }
            limits.min = limits.min.min(Vec2::splat(1_000_000.0));
            limits.max = limits.max.max(limits.min);
            limits
        }
        Node::Split { axis, first, second, .. } => {
            let a = node_limits(first, style, content);
            let b = node_limits(second, style, content);
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

pub(super) fn clamp_float(rect: Rect, screen: Rect) -> Rect {
    crate::docking::chrome::clamp(rect, screen)
}

pub(super) fn split_bounds(available: f32, axis: SplitAxis, first: PanelLimits, second: PanelLimits) -> (f32, f32) {
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
