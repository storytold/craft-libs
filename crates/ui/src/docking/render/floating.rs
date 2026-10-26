use std::{fmt::Debug, hash::Hash};

use egui::{Context, Id, Key, Pos2, Rect, Ui, pos2, vec2};

use super::{GroupKind, Renderer, clamp_float, node_limits};
use crate::docking::chrome::{ChromeStyle, FloatingFrame, RESIZE_BAND, TitleBar};
use crate::docking::{Action, DropProposal, Floating, GroupDrag, GroupGesture, Node, PanelLimits, Placement, Target};

pub(in crate::docking) use crate::docking::chrome::RESIZE_HANDLES;

/// A floating window as drawn: what covers the layout behind it.
pub(in crate::docking) struct WindowHit<P> {
    pub(super) window: Rect,
    /// Where the window's tree is drawn.
    pub(super) body: Rect,
    pub(super) members: Vec<P>,
}

impl<P: Clone + Eq + Hash + Debug + Send + Sync + 'static> Renderer<'_, P> {
    pub(in crate::docking) fn floating(&mut self, ctx: &Context, windows: &[Floating<P>]) {
        let screen = self.float_bounds;
        if !crate::docking::targets::usable(screen) {
            return;
        }
        let layers = ctx.memory(|memory| memory.layer_ids().collect::<Vec<_>>());
        let mut windows: Vec<_> = windows.iter().collect();
        windows.sort_by_key(|window| {
            let id = window.panels().next().map(|panel| self.id.with(("floating", panel)));
            layers.iter().position(|layer| Some(layer.id) == id).unwrap_or(usize::MAX)
        });
        #[cfg(feature = "native-windows")]
        let windows = self.native_windows(ctx, windows);
        for window in windows {
            self.floating_window(ctx, window, screen);
        }
    }

    /// Size limits of a floating window: its title bar and, unless it is minimized, its tree.
    pub(super) fn window_limits(&self, window: &Floating<P>) -> PanelLimits {
        let chrome = self.window_chrome(window);
        let tree = node_limits(&window.root, self.style, &*self.content);
        let body = match window.minimized {
            true => (self.minimized_body(window), self.minimized_body(window)),
            false => (tree.min.y, tree.max.y),
        };
        let min = vec2(tree.min.x, chrome + body.0).min(vec2(1_000_000.0, 1_000_000.0));
        PanelLimits { min, max: vec2(tree.max.x, chrome + body.1).max(min).min(vec2(1_000_000.0, 1_000_000.0)) }
    }

    /// The height of a minimized window's body: the tab strip of its only group, or nothing.
    pub(super) fn minimized_body(&self, window: &Floating<P>) -> f32 {
        if matches!(window.root, Node::Tabs { .. }) { self.style.tab_height } else { 0.0 }
    }

    /// The title bar and, for a movable (resizable) window, the resize band above it.
    fn window_chrome(&self, window: &Floating<P>) -> f32 {
        let band = if window.panels().all(|panel| self.permissions(panel).movable) { RESIZE_BAND } else { 0.0 };
        band + self.style.title_height
    }

    /// Where a floating window draws its tree, as [`Self::floating_window`] places it.
    pub(super) fn window_body(&self, window: &Floating<P>) -> Rect {
        let [x, y, width, height] = window.rect;
        let limits = self.window_limits(window);
        let size = vec2(width, height).max(limits.min).min(limits.max);
        let rect = crate::docking::chrome::clamp(Rect::from_min_size(pos2(x, y), size), self.float_bounds);
        Rect::from_min_max(pos2(rect.left(), (rect.top() + self.window_chrome(window)).min(rect.bottom())), rect.max)
    }

    fn floating_window(&mut self, ctx: &Context, window: &Floating<P>, screen: Rect) {
        let Some(first) = window.panels().next() else { return };
        let [x, y, width, height] = window.rect;
        let limits = self.window_limits(window);
        let size = vec2(width, height).max(limits.min).min(limits.max);
        let rect = crate::docking::chrome::clamp(Rect::from_min_size(pos2(x, y), size), screen);
        let id = self.id.with(("floating", first));
        if self.activation.as_ref().is_some_and(|panel| window.contains(panel)) {
            ctx.move_to_top(FloatingFrame::layer(id));
        }
        let movable = window.panels().all(|panel| self.permissions(panel).movable);
        let style = ChromeStyle::from_dock(self.style);
        let resize_label = self.style.resize_window_label.clone();
        let opacity = self.drag_opacity(ctx, first);
        let response =
            FloatingFrame::new(id, rect).bounds(screen).limits(limits).resizable(movable).resize_label(&resize_label).opacity(opacity).show(
                ctx,
                &style,
                |ui, contents| self.window_contents(ui, window, contents, rect, &style),
            );
        for handle in &response.handles {
            self.register_interaction(ctx, handle.id);
        }
        let down = ctx.input(|i| i.pointer.primary_down());
        ctx.data_mut(|data| {
            let key = self.id.with("pointer-resize");
            if let Some(started) = response.resize_started {
                data.insert_temp(key, started);
            } else if !down && data.get_temp::<Id>(key).is_some_and(|id| response.handles.iter().any(|h| h.id == id)) {
                data.remove::<Id>(key);
            }
        });
        if let Some(resized) = response.resized {
            self.output
                .actions
                .push(Action::MoveFloating { panel: first.clone(), rect: [resized.left(), resized.top(), resized.width(), resized.height()] });
        }
    }

    /// Opacity of the in-window floating window whose first panel is `first`: faded
    /// ([`super::DRAG_OPACITY`]) while its title bar drag highlights a drop target, as the last
    /// frame drew it ([`super::preview::GROUP_HIGHLIGHTED`]), else opaque. Its tabs and drag
    /// bars only drag a ghost, so the window stays opaque for them.
    fn drag_opacity(&self, ctx: &Context, first: &P) -> f32 {
        let moving = ctx
            .data(|d| d.get_temp::<GroupDrag<P>>(self.id.with("group-drag")))
            .is_some_and(|drag| drag.gesture == GroupGesture::Window && drag.panel == *first);
        let highlighted = ctx.data(|d| d.get_temp::<bool>(self.id.with(super::preview::GROUP_HIGHLIGHTED))).unwrap_or(false);
        if moving && highlighted { super::DRAG_OPACITY } else { 1.0 }
    }

    /// The title bar, then the window's tree below it, in `contents`. Moves carry the whole
    /// `frame`, resize band included.
    fn window_contents(&mut self, ui: &mut Ui, window: &Floating<P>, contents: Rect, frame: Rect, style: &ChromeStyle) {
        let title = Rect::from_min_max(contents.min, pos2(contents.right(), (contents.top() + self.style.title_height).min(contents.bottom())));
        self.title_bar(ui, window, title, frame, style);
        let body = Rect::from_min_max(pos2(contents.left(), title.bottom()), contents.max);
        self.floating_body(ui, window, body, frame);
    }

    /// A floating window's tree in `body`, like the docked layout's: splits, groups, dividers
    /// and intersections. A minimized window shows the tab strip of its only group, or nothing.
    pub(super) fn floating_body(&mut self, ui: &mut Ui, window: &Floating<P>, body: Rect, frame: Rect) {
        let Some(key) = window.panels().next().cloned() else { return };
        match (&window.root, window.minimized) {
            (Node::Tabs { panels, active }, true) => self.group(ui, panels, *active, true, body, GroupKind::Floating { window: frame }),
            (_, true) => {}
            (root, false) => {
                let outer = self.tree.replace((key, frame));
                self.node(ui, root, body, &mut Vec::new(), (root, body));
                self.tree = outer;
            }
        }
        self.frame.windows.push(WindowHit { window: frame, body, members: window.panels().cloned().collect() });
    }

    /// Drag moves the whole window live, double-click minimizes or expands it, × closes every
    /// panel in it.
    fn title_bar(&mut self, ui: &mut Ui, window: &Floating<P>, title: Rect, rect: Rect, style: &ChromeStyle) {
        let Some(first) = window.panels().next() else { return };
        let movable = window.panels().all(|p| self.permissions(p).movable);
        let closable = self.style.close_button && window.panels().all(|p| self.permissions(p).closable);
        let (move_label, close_label) = (self.style.move_window_label.clone(), self.style.close_window_label.clone());
        let mut bar = TitleBar::new(self.id.with(("move-group", Some(first))), &move_label).movable(movable);
        if closable {
            bar = bar.close(&close_label);
        }
        let response = bar.show(ui, title, style);
        self.register_interaction(ui.ctx(), response.bar.id);
        if movable && response.bar.drag_started() {
            let origin = ui.input(|i| i.pointer.press_origin()).or_else(|| ui.ctx().pointer_interact_pos()).unwrap_or(title.center());
            let drag = GroupDrag {
                panel: first.clone(),
                original: [rect.left(), rect.top(), rect.width(), rect.height()],
                gesture: GroupGesture::Window,
                members: window.panels().cloned().collect(),
                origin,
                source: rect,
            };
            ui.ctx().data_mut(|d| d.insert_temp(self.id.with("group-drag"), drag));
        }
        if movable
            && response.bar.dragged()
            && let Some(drag) = ui.ctx().data(|d| d.get_temp::<GroupDrag<P>>(self.id.with("group-drag")))
            && drag.gesture == GroupGesture::Window
            && let Some(pointer) = ui.ctx().pointer_interact_pos()
            && !ui.input(|i| i.key_pressed(Key::Escape))
            && !crate::focus::lost(ui.ctx())
        {
            // From the press origin, so fractional pointer steps do not accumulate rounding.
            let moved = clamp_float(drag.source.translate(pointer - drag.origin), self.float_bounds);
            self.output.actions.push(Action::MoveFloating { panel: first.clone(), rect: [moved.left(), moved.top(), moved.width(), moved.height()] });
        }
        if response.bar.double_clicked() {
            self.output.actions.push(Action::SetMinimized { panel: first.clone(), minimized: !window.minimized });
        }
        if let Some(close) = response.close {
            self.register_interaction(ui.ctx(), close.id);
            if close.clicked() {
                self.output.actions.push(Action::CloseWindow { panel: first.clone() });
            }
        }
    }

    /// The title bar of a floating window occludes what lies behind it: a no-op proposal. Then
    /// the layout's outer edge (P8). None leaves the pointer to the group targets. `skip_own`
    /// looks through the window holding `members` (a window moving live).
    pub(super) fn extra_proposal(&self, members: &[P], pointer: Pos2, skip_own: bool) -> Option<DropProposal<P>> {
        let own = |panels: &[P]| panels.iter().any(|p| members.contains(p));
        let window = self.frame.windows.iter().rev().find(|w| w.window.contains(pointer) && !(skip_own && own(&w.members)));
        if let Some(window) = window {
            let in_group = self.targets.iter().any(|t| t.rect.contains(pointer) && t.tabs.iter().any(|(p, _)| window.members.contains(p)));
            if in_group {
                return None;
            }
            let anchor = members.first()?.clone();
            return Some(DropProposal { anchor, placement: Placement::Tab { before: None }, preview: Rect::NOTHING, insertion: None, noop: true });
        }
        let (anchor, zone, preview) = self.edge_proposal(members, pointer)?;
        let lead = members.first()?.clone();
        let action = match members {
            [_] => Action::Move { panel: lead, anchor: anchor.clone(), placement: Placement::Edge(zone) },
            _ => Action::MoveGroup { panel: lead, to: Target::At { anchor: anchor.clone(), placement: Placement::Edge(zone) } },
        };
        if !self.leaves_room(action) {
            return None;
        }
        Some(DropProposal { anchor, placement: Placement::Edge(zone), preview, insertion: None, noop: false })
    }

    /// What a group drag carries: the whole window rect for the only group of a floating
    /// window, so floating it again keeps the window's size; else the group's rect.
    pub(super) fn drag_source(&self, owner: &super::group::TabGroup<'_, P>) -> Rect {
        match owner.kind {
            GroupKind::Floating { window } if owner.panels.first().is_some_and(|p| self.whole_window(p)) => window,
            _ => owner.rect,
        }
    }

    /// Whether a double-click or the group menu minimizes `owner`: a dock group, or the only
    /// group of a floating window, which minimizes the window to that group's strip.
    pub(super) fn minimizes(&self, owner: &super::group::TabGroup<'_, P>) -> bool {
        match owner.kind {
            GroupKind::Dock => true,
            GroupKind::Floating { .. } => self.whole_window(owner.selected),
            GroupKind::Frame => false,
        }
    }

    /// Whether `panel`'s group is the whole tree of a floating window.
    pub(super) fn whole_window(&self, panel: &P) -> bool {
        matches!(self.place(panel), Some(crate::docking::dock_model::Place::Floating { path, .. }) if path.is_empty())
    }

    /// Whether moving a whole window docks it: [`crate::docking::DockStyle::title_bar_docks`],
    /// or a native window moved by its only tab or group.
    pub(super) fn title_bar_docks(&self) -> bool {
        #[cfg(feature = "native-windows")]
        if self.cross.docks {
            return true;
        }
        self.style.title_bar_docks
    }

    /// Release of a title bar drag. The window already sits where it was dragged; a dock or
    /// group target docks or merges every group in it, else hosts get the release in
    /// `group_drop`.
    pub(super) fn drop_window(&mut self, drag: &GroupDrag<P>, pointer: Pos2, float_only: bool) {
        let float_only = float_only || !self.title_bar_docks();
        if !float_only && let Some((action, _)) = self.dock_group_proposal(drag, pointer) {
            self.output.actions.extend(action);
            return;
        }
        let proposal = if float_only { None } else { self.group_proposal(drag, pointer) };
        match proposal.filter(|p| !p.noop) {
            Some(proposal) => {
                let to = Target::At { anchor: proposal.anchor, placement: proposal.placement };
                self.output.actions.push(super::dock::group_action(drag, to));
            }
            None => self.output.group_drop = Some((drag.panel.clone(), pointer)),
        }
    }
}
