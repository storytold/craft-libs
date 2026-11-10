use std::{collections::HashMap, fmt::Debug, hash::Hash};

use egui::{Context, Key, Pos2, Rect, Ui, Vec2, pos2, vec2};

use super::{DROP_MARGIN, Drawn, Renderer, clamp_float, dock::group_action};
use crate::docking::chrome::drop_preview;
use crate::docking::{
    Action, Drag, DropProposal, DropRequest, DropTarget, GroupDrag, GroupGesture, Layout, Node, Placement, Target, Zone, cancel_drag,
    dock_model::{Place, sized_split_depth},
    surface::TAB_PADDING,
    targets::{DockDrop, SIDE_EDGE_ZONE_DEPTH, resolve_drop_on},
};
use crate::layout::{SplitAxis, SplitSize};

/// Ctrl (Windows, Linux) or Cmd (macOS) held: the drop floats and no target is offered.
fn float_only(ctx: &Context) -> bool {
    ctx.input(|i| i.modifiers.command || i.modifiers.ctrl || i.modifiers.mac_cmd)
}

/// Share of the accent that fills a tab drop's outline: light, so it does not read as a line.
const TAB_FILL: f32 = 0.08;

/// The preview of a drop that joins a group as a tab, or of the rect a drop beside something
/// gives the moved panels: a 2 pt accent outline inside `rect`, with a light fill.
fn tab_outline(painter: &egui::Painter, rect: Rect, accent: egui::Color32) {
    painter.rect_filled(rect, 0.0, accent.gamma_multiply(TAB_FILL));
    painter.rect_stroke(rect, 0.0, egui::Stroke::new(2.0, accent), egui::StrokeKind::Inside);
}

/// Height of a panel floated out of a dock column when the host gives no floating size.
const DOCKED_FLOAT_HEIGHT: f32 = 320.0;

/// Least height of a floating window or flyout fitted to a short panel's content.
pub(super) const MIN_FITTED_HEIGHT: f32 = 120.0;

/// Key of whether the area's group drag highlighted a drop target on the last frame, under the
/// area's id. An in-window floating window that its bar drag moves fades from it on the next
/// frame: the window draws before the preview.
pub(super) const GROUP_HIGHLIGHTED: &str = "group-drag-highlighted";

/// What finishing a drag reads of the pointer, in the coordinates of the window finishing it.
#[derive(Clone, Copy, Debug)]
pub(super) struct DragInput {
    pub pointer: Option<Pos2>,
    pub down: bool,
    pub released: bool,
    /// The window kept its focus: false on the pass it loses it (see [`crate::focus`]).
    pub focused: bool,
    /// The pointer left the window.
    pub gone: bool,
    pub escape: bool,
    /// Ctrl or Cmd held.
    pub command: bool,
    /// A touch ended this frame.
    pub touch_ended: bool,
}

impl DragInput {
    /// This window's own input.
    fn of(ctx: &Context) -> Self {
        let command = float_only(ctx);
        let focus_lost = crate::focus::lost(ctx);
        ctx.input(|i| Self {
            pointer: i.pointer.interact_pos().filter(|p| p.is_finite()),
            down: i.pointer.primary_down(),
            released: i.pointer.primary_released(),
            focused: !focus_lost,
            gone: !i.pointer.has_pointer(),
            escape: i.key_pressed(Key::Escape),
            command,
            touch_ended: i.events.iter().any(|e| matches!(e, egui::Event::Touch { phase: egui::TouchPhase::End, .. })),
        })
    }
}

impl<P: Clone + Eq + Hash + Debug + Send + Sync + 'static> Renderer<'_, P> {
    pub(in crate::docking) fn finish_group_drag(&mut self, ui: &Ui) {
        let ctx = ui.ctx();
        ctx.data_mut(|d| d.remove::<bool>(self.id.with(GROUP_HIGHLIGHTED)));
        let key = self.id.with("group-drag");
        let Some(drag) = ctx.data(|d| d.get_temp::<GroupDrag<P>>(key)) else { return };
        if !drag.members.iter().all(|p| self.layout.contains(p) && self.permissions(p).movable) {
            ctx.data_mut(|d| d.remove::<GroupDrag<P>>(key));
            return;
        }
        let input = DragInput::of(ctx);
        #[cfg(feature = "native-windows")]
        let Some((drag, input)) = self.cross_group(ui, &drag, input) else { return };
        let DragInput { escape, focused, down, released, pointer, .. } = input;
        let ghost = pointer.map(|pointer| self.group_ghost(&drag, pointer));
        if !escape
            && focused
            && (down || released)
            && let Some(pointer) = pointer
            && !self.blockers.iter().any(|r| r.contains(pointer))
            && self.host_drop(ctx, &drag.panel, pointer, true, released, ghost.filter(|_| drag.gesture != GroupGesture::Window))
        {
            if released {
                ctx.data_mut(|d| d.remove::<GroupDrag<P>>(key));
            }
            return;
        }
        // A button release outside the window may never reach egui.
        let has_pointer = !input.gone;
        // Touch-up also removes the pointer, but still completes a drop.
        let touch_ended = input.touch_ended;
        let float_only = input.command;
        if escape || !focused || !down || !has_pointer {
            cancel_drag::<P>(ctx, self.id);
            if escape || !focused {
                if drag.gesture == GroupGesture::Window {
                    self.output.actions.push(Action::MoveFloating { panel: drag.panel, rect: drag.original });
                }
                return;
            }
            // Leaving the window ends a group drag where it is, with no drop, unless only a
            // release outside the window floats.
            let gone = !has_pointer && !touch_ended;
            let floats_outside = !self.style.float_on_empty_drop && drag.gesture != GroupGesture::Window;
            if !(released && !gone) && !(gone && floats_outside) {
                return;
            }
            let Some(pointer) = pointer.filter(|p| !self.blockers.iter().any(|r| r.contains(*p))) else { return };
            let outside = gone || !ctx.content_rect().contains(pointer);
            match drag.gesture {
                GroupGesture::Window => self.drop_window(&drag, pointer, float_only),
                GroupGesture::Group | GroupGesture::Column => self.drop_group(&drag, pointer, float_only || outside),
            }
        } else if let Some(pointer) = pointer.filter(|p| !self.blockers.iter().any(|r| r.contains(*p))) {
            self.output.group_drag = Some(drag.panel.clone());
            if self.paint_group_drag(ui, &drag, pointer, float_only) {
                ctx.data_mut(|d| d.insert_temp(self.id.with(GROUP_HIGHLIGHTED), true));
            }
        }
    }

    /// The valid target under the pointer for a whole group. A target holding the group
    /// itself is a no-op proposal, and so is a drop that would change only sizes; a target
    /// the model would reject is no target. A drag that takes a whole floating window along
    /// looks through it ([`Self::looks_through`]). A window moving live does not split frames
    /// or, moved by its bar, join a body's center.
    pub(super) fn group_proposal(&self, drag: &GroupDrag<P>, pointer: Pos2) -> Option<DropProposal<P>> {
        if let Some(proposal) = self.extra_proposal(&drag.members, pointer, self.looks_through(drag)) {
            return self.valid_group(drag, self.settled(proposal, |to| group_action(drag, to)));
        }
        let target = self.group_target(drag, pointer)?;
        self.group_proposal_at(drag, pointer, target)
    }

    /// Whether a group drag looks through the floating window it comes from: the window's
    /// title bar moves it, or the drag holds the window's only group and so takes the window
    /// along. The window then covers nothing behind it.
    pub(super) fn looks_through(&self, drag: &GroupDrag<P>) -> bool {
        match drag.gesture {
            GroupGesture::Window => true,
            GroupGesture::Group => self.whole_window(&drag.panel),
            GroupGesture::Column => false,
        }
    }

    /// The group under the pointer that a group drag addresses, past the drag's own groups
    /// when it looks through its window.
    fn group_target(&self, drag: &GroupDrag<P>, pointer: Pos2) -> Option<&DropTarget<P>> {
        let through = self.looks_through(drag);
        let own = |t: &DropTarget<P>| t.tabs.iter().any(|(p, _)| drag.members.contains(p));
        self.targets.iter().rev().find(|t| crate::docking::targets::usable(t.rect) && t.rect.contains(pointer) && !(through && own(t)))
    }

    /// [`Self::group_proposal`] on `target` alone.
    fn group_proposal_at(&self, drag: &GroupDrag<P>, pointer: Pos2, target: &DropTarget<P>) -> Option<DropProposal<P>> {
        let window = drag.gesture == GroupGesture::Window;
        if target.tabs.iter().any(|(p, _)| drag.members.contains(p)) {
            return Some(DropProposal {
                anchor: drag.panel.clone(),
                placement: Placement::Tab { before: None },
                preview: target.rect,
                insertion: None,
                noop: true,
            });
        }
        let proposal = self.resolve_roomy(&drag.panel, pointer, std::slice::from_ref(target), |to| group_action(drag, to))?;
        // A window moving live splits only groups of other floating windows. A native
        // window's bar also takes the main window's frame edges, but not a body's center. A
        // native window moved by its only tab or group joins a body's center too, as that tab
        // or group drag would.
        let into_window = matches!(self.place(&proposal.anchor), Some(Place::Floating { .. }));
        #[cfg(feature = "native-windows")]
        let (edges, joins) = (self.cross.bar, self.cross.docks);
        #[cfg(not(feature = "native-windows"))]
        let (edges, joins) = (false, false);
        let split = match proposal.placement {
            Placement::Split(Zone::Center) => !joins,
            Placement::Split(_) | Placement::SplitSized { .. } => !edges,
            _ => false,
        };
        if window && !into_window && split {
            return None;
        }
        self.valid_group(drag, proposal)
    }

    /// `proposal` when it is a no-op or the model takes it for the whole group.
    fn valid_group(&self, drag: &GroupDrag<P>, proposal: DropProposal<P>) -> Option<DropProposal<P>> {
        let to = Target::At { anchor: proposal.anchor.clone(), placement: proposal.placement.clone() };
        (proposal.noop || self.layout.clone().apply_with_permissions(group_action(drag, to), |p| self.permissions(p)).is_ok()).then_some(proposal)
    }

    /// [`crate::docking::resolve_drop_by`] with [`crate::docking::DockStyle::zone_rule`] and the
    /// canvas sides of [`crate::docking::DockStyle::drop_edges`] (none in the docking profile),
    /// where a split that would leave a group without room becomes a join of the target group,
    /// or no target when the group takes no tabs. A drop that would change only sizes is a no-op ([`Self::settled`]).
    fn resolve_roomy(&self, panel: &P, pointer: Pos2, targets: &[DropTarget<P>], action: impl Fn(Target<P>) -> Action<P>) -> Option<DropProposal<P>> {
        let proposal = resolve_drop_on(panel, pointer, targets, self.style.zone_rule(), self.style.canvas_sides())?;
        if !matches!(proposal.placement, Placement::Split(zone) if zone != Zone::Center) {
            return Some(self.settled(proposal, action));
        }
        if self.leaves_room(action(Target::At { anchor: proposal.anchor.clone(), placement: proposal.placement.clone() })) {
            return Some(self.settled(proposal, action));
        }
        if let Some(placement) = self.sized_split(panel, &proposal, &action)
            && self.leaves_room(action(Target::At { anchor: proposal.anchor.clone(), placement: placement.clone() }))
        {
            return Some(self.settled(DropProposal { placement, ..proposal }, action));
        }
        let target = targets.iter().rev().find(|t| crate::docking::targets::usable(t.rect) && t.rect.contains(pointer))?;
        let proposal = resolve_drop_on(
            panel,
            pointer,
            std::slice::from_ref(&DropTarget { allow_splits: false, ..target.clone() }),
            self.style.zone_rule(),
            self.style.canvas_sides(),
        )?;
        Some(self.settled(proposal, action))
    }

    /// The side split of `proposal`'s frame when halving it leaves no room
    /// ([`Placement::SplitSized`]): the frame keeps its size, and the frames beyond the divider
    /// on that side, or on the opposite side for a frame at the edge of its area, give the new
    /// frame its minimum (with [`DROP_MARGIN`]). A floating panel
    /// keeps its size along the split instead, and one in a dock column its column's width or
    /// its group's height, when that fits. `None` without such a divider, or when those frames
    /// cannot give the new frame its minimum.
    fn sized_split(&self, panel: &P, proposal: &DropProposal<P>, action: &impl Fn(Target<P>) -> Action<P>) -> Option<Placement<P>> {
        let Placement::Split(zone) = proposal.placement else { return None };
        let axis = if matches!(zone, Zone::Top | Zone::Bottom) { SplitAxis::Vertical } else { SplitAxis::Horizontal };
        let along = |v: Vec2| if axis == SplitAxis::Horizontal { v.x } else { v.y };
        let start = matches!(zone, Zone::Left | Zone::Top);
        let anchor = &proposal.anchor;
        let now = self.drawn_groups(self.layout);
        let frame = along(now.iter().find(|(node, _)| holds(node, anchor))?.1.size());
        let mut candidate = self.layout.clone();
        let halves = action(Target::At { anchor: anchor.clone(), placement: proposal.placement.clone() });
        candidate.apply_with_permissions(halves, |p| self.permissions(p)).ok()?;
        let (root, area, path) = match candidate.place(anchor)? {
            Place::Node(path) => (candidate.root.as_ref()?, self.workspace, path),
            Place::Floating { window, path } => {
                let window = candidate.floating.get(window)?;
                (&window.root, self.window_body(window), path)
            }
            Place::Dock { .. } => return None,
        };
        // The splits from the root down to the new one, as drawn with halves.
        let sizer = self.sizer();
        let (_, split_path) = path.split_last()?;
        let mut chain = vec![(root, area)];
        for &second in split_path {
            let &(node, rect) = chain.last()?;
            let [a, b] = sizer.children(node, rect)?;
            chain.push(if second { b } else { a });
        }
        let &(Node::Split { first, second, .. }, halved) = chain.last()? else { return None };
        let new = if start { first } else { second };
        let depth =
            sized_split_depth(split_path, zone, |d| chain.get(d).is_some_and(|(n, _)| matches!(n, Node::Split { axis: own, .. } if *own == axis)))?;
        // The frame is in the outer split's second child when the divider is on its start side.
        let in_second = *split_path.get(depth)?;
        let &(outer, outer_rect) = chain.get(depth)?;
        let Node::Split { size: outer_size, first: a, second: b, .. } = outer else { return None };
        if matches!(**a, Node::Dock { .. }) || matches!(**b, Node::Dock { .. }) {
            return None;
        }
        let [a, b] = sizer.children(outer, outer_rect)?;
        let far = if in_second { a } else { b };
        // The far side's edge at the divider is its end edge when the frame is in the second
        // child.
        let room = sizer.travel(far.0, far.1, axis, in_second).0 - DROP_MARGIN;
        let least = along(super::node_limits(new, self.style, &*self.content).min) + DROP_MARGIN;
        let gap = self.style.gap;
        // A split frame's width is its share of the room, not one of its own.
        let saved = match self.place(panel) {
            Some(Place::Floating { .. }) => now.iter().find(|(node, _)| holds(node, panel)).map(|(_, rect)| along(rect.size())),
            Some(place @ Place::Dock { .. }) if axis == SplitAxis::Horizontal => self.layout.column(place).map(|column| column.width),
            Some(place @ Place::Dock { .. }) => self.layout.group(place).and_then(|group| group.height),
            _ => None,
        };
        let most = along(halved.size()) + room - frame - gap;
        if least > most {
            return None;
        }
        let size = saved.filter(|width| (least..=most).contains(width)).unwrap_or(least);
        // The anchor's side of the outer split grows by what the halves did not take already.
        let grow = frame + gap + size - along(halved.size());
        let first = along(a.1.size()) + if in_second { -grow } else { grow };
        let outer = super::sizes::resized(*outer_size, first, along(a.1.size()) + along(b.1.size()))?;
        let size = super::sizes::resized(SplitSize::Ratio(0.5), if start { size } else { frame }, frame + size)?;
        Some(Placement::SplitSized { zone, size, outer })
    }

    /// Every group of `layout`'s trees as drawn, with its rect.
    fn drawn_groups<'l>(&self, layout: &'l Layout<P>) -> Vec<(&'l Node<P>, Rect)> {
        let sizer = self.sizer();
        let mut out = Vec::new();
        for (root, area) in self.trees(layout) {
            let mut drawn = Vec::new();
            sizer.groups(root, area, &mut drawn);
            out.extend(drawn.into_iter().map(|d| (d.node, d.rect)));
        }
        out
    }

    /// `proposal`, marked a no-op when `action` at it would leave every panel where it is,
    /// changing only sizes ([`Layout::same_arrangement`]): a lone group dropped beside the
    /// group it already borders, or a tab on the edge its frame already holds. Such a drop
    /// shows no target and changes nothing.
    fn settled(&self, mut proposal: DropProposal<P>, action: impl Fn(Target<P>) -> Action<P>) -> DropProposal<P> {
        if !proposal.noop {
            let mut candidate = self.layout.clone();
            let to = Target::At { anchor: proposal.anchor.clone(), placement: proposal.placement.clone() };
            proposal.noop = candidate.apply_with_permissions(action(to), |p| self.permissions(p)).is_ok() && candidate.same_arrangement(self.layout);
        }
        proposal
    }

    /// Whether `action`, a drop that splits a group or the outer edge, leaves room: every group
    /// it makes smaller keeps its minimum, and each divider can still travel both ways. A group
    /// within [`DROP_MARGIN`] of its minimum along an axis passes when the dividers along its
    /// sides can travel, the groups beyond it giving way as they do for a dragged divider, so a
    /// narrow group still splits when its neighbours have room for both halves. An action the
    /// model rejects is left to the model's check.
    pub(super) fn leaves_room(&self, action: Action<P>) -> bool {
        let mut candidate = self.layout.clone();
        if candidate.apply_with_permissions(action, |p| self.permissions(p)).is_err() {
            return true;
        }
        let sizer = self.sizer();
        let drawn = |root: &Node<P>, area: Rect| {
            let mut out: Vec<Drawn<'_, P>> = Vec::new();
            sizer.groups(root, area, &mut out);
            out.into_iter().filter_map(|d| Some((lead(d.node)?.clone(), d.rect, d.limits.min))).collect::<Vec<_>>()
        };
        let before: HashMap<P, Vec2> =
            self.trees(self.layout).into_iter().flat_map(|(root, area)| drawn(root, area)).map(|(panel, rect, _)| (panel, rect.size())).collect();
        self.trees(&candidate).into_iter().all(|(root, area)| {
            drawn(root, area).into_iter().all(|(panel, rect, min)| {
                let (size, room) = (rect.size(), rect.size() - min);
                let fits = |room: f32, axis| room >= DROP_MARGIN || sizer.frees((root, area), rect, axis);
                before.get(&panel).is_some_and(|old| size.x >= old.x - 0.5 && size.y >= old.y - 0.5)
                    || (room.min_elem() > -0.01 && fits(room.x, SplitAxis::Horizontal) && fits(room.y, SplitAxis::Vertical))
            })
        })
    }

    /// The trees of `layout` as drawn: the docked root in the workspace and each open floating
    /// window's tree in its body.
    fn trees<'l>(&self, layout: &'l Layout<P>) -> Vec<(&'l Node<P>, Rect)> {
        let windows = layout.floating.iter().filter(|w| !w.minimized).map(|w| (&w.root, self.window_body(w)));
        layout.root.iter().map(|root| (root, self.workspace)).chain(windows).collect()
    }

    /// Release of a group drag: join or split at the target, or float at the ghost with the
    /// group's size. Over its own docked position, or where the drop would change only sizes,
    /// nothing changes; a floating group released over itself or a window's title bar moves
    /// there.
    fn drop_group(&mut self, drag: &GroupDrag<P>, pointer: Pos2, float_only: bool) {
        if pointer.distance(drag.origin) < 2.0 {
            return;
        }
        if !float_only && let Some((action, _)) = self.dock_group_proposal(drag, pointer) {
            self.output.actions.extend(action);
            return;
        }
        let floating = matches!(self.place(&drag.panel), Some(Place::Floating { .. }));
        let proposal = if float_only { None } else { self.group_proposal(drag, pointer) };
        if let Some(proposal) = proposal.as_ref().filter(|p| !p.noop) {
            let to = Target::At { anchor: proposal.anchor.clone(), placement: proposal.placement.clone() };
            self.output.actions.push(group_action(drag, to));
            return;
        }
        // A no-op anchored on another group would change only sizes.
        if proposal.is_some_and(|p| !floating || !drag.members.contains(&p.anchor)) {
            return;
        }
        if !float_only && !self.style.float_on_empty_drop {
            return;
        }
        // No target: hosts that still apply their own policy for a whole-group release get it.
        self.output.group_drop = Some((drag.panel.clone(), pointer));
        let rect = self.group_ghost(drag, pointer);
        let action = group_action(drag, Target::Float { rect: Some([rect.left(), rect.top(), rect.width(), rect.height()]) });
        if self.layout.clone().apply_with_permissions(action.clone(), |p| self.permissions(p)).is_ok() {
            self.output.actions.push(action);
        }
    }

    /// The ghost at the pointer (a window moving live is its own ghost) under one destination
    /// preview. The ghost has a translucent fill, a 2 pt outline and the members' labels.
    /// Returns whether it highlighted a drop target.
    pub(super) fn paint_group_drag(&mut self, ui: &Ui, drag: &GroupDrag<P>, pointer: Pos2, float_only: bool) -> bool {
        let ctx = ui.ctx();
        let painter = self.preview_painter(ctx);
        if drag.gesture != GroupGesture::Window {
            let ghost = self.group_ghost(drag, pointer);
            drop_preview(&painter, ghost, self.style.accent, true);
            let strip = Rect::from_min_size(ghost.min, vec2(ghost.width(), self.style.tab_height.min(ghost.height())));
            let labels = painter.with_clip_rect(strip.shrink(2.0));
            let mut x = strip.left() + TAB_PADDING * 0.5;
            for panel in &drag.members {
                let width = strip.right() - TAB_PADDING * 0.5 - x;
                if width <= 0.0 {
                    break;
                }
                let label = self.label_of(panel);
                let galley = self.tab_label(ui, panel, &label, self.style.text, width);
                let advance = galley.size().x + TAB_PADDING;
                labels.galley(pos2(x, strip.center().y - galley.size().y * 0.5), galley, self.style.text);
                x += advance;
            }
        }
        if float_only || (drag.gesture == GroupGesture::Window && !self.title_bar_docks()) {
            return false;
        }
        if let Some((action, drop)) = self.dock_group_proposal(drag, pointer) {
            let highlighted = action.is_some();
            if let Some(action) = action {
                self.paint_dock_preview(ctx, &painter, action, &drag.members, &drop);
                if drag.gesture != GroupGesture::Window {
                    self.paint_float_hint(ctx, pointer);
                }
            }
            return highlighted;
        }
        let proposal = self.group_proposal(drag, pointer).filter(|p| !p.noop);
        let zones = self.paint_group_zones(&painter, drag, pointer, proposal.as_ref());
        if let Some(proposal) = proposal {
            if zones {
                self.paint_marker(&painter, &proposal);
            } else {
                let split = matches!(proposal.placement, Placement::Split(zone) | Placement::SplitSized { zone, .. } | Placement::Edge(zone) if zone != Zone::Center);
                self.paint_preview(ctx, &painter, &proposal, split, &drag.members, |to| group_action(drag, to));
            }
            if drag.gesture != GroupGesture::Window {
                self.paint_float_hint(ctx, pointer);
            }
            return true;
        }
        false
    }

    pub(in crate::docking) fn finish_drag(&mut self, ctx: &Context) {
        let Some(drag) = ctx.data(|d| d.get_temp::<Drag<P>>(self.id.with("drag"))) else { return };
        let escape = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
        let input = DragInput { escape, ..DragInput::of(ctx) };
        #[cfg(feature = "native-windows")]
        let Some((drag, input)) = self.cross_tab(ctx, &drag, input) else { return };
        let DragInput { pointer, down, released, focused, gone, escape, .. } = input;
        // Leaving the window is the drop: on X11 and Windows the release outside never reaches
        // egui, so the drag would otherwise stay on the pointer. `interact_pos` still holds the
        // last position this frame, and the panel floats there rather than docking at the edge.
        // With native viewports the release does arrive, and `gone` is false.
        let released = released || gone;
        if escape || !focused || !self.layout.contains(&drag.panel) || (!down && !released) {
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
        let float_only = input.command || drag.undock;
        let outside = gone || !ctx.content_rect().contains(pointer);
        let empty_drop =
            self.style.float_on_empty_drop && (!drag.source.contains(pointer) || self.layout.floating.iter().any(|g| g.contains(&drag.panel)));
        let floatable = self.permissions(&drag.panel).floatable && (float_only || outside || empty_drop);
        let floating_rect = floatable.then(|| self.tab_float_rect(&drag, pointer));
        if self.host_drop(ctx, &drag.panel, pointer, false, released, floating_rect) {
            if released {
                cancel_drag::<P>(ctx, self.id);
            }
            return;
        }
        let dock = if float_only || outside { None } else { self.dock_tab_proposal(&drag.panel, pointer) };
        if let Some((action, drop)) = dock {
            if released {
                cancel_drag::<P>(ctx, self.id);
                if let Some(action) = action.filter(|_| pointer.distance(drag.origin) >= 2.0) {
                    self.output.actions.push(action);
                    self.output.focused = Some(drag.panel);
                }
            } else if let Some(action) = action {
                let painter = self.preview_painter(ctx);
                self.paint_dock_preview(ctx, &painter, action, std::slice::from_ref(&drag.panel), &drop);
                self.paint_float_hint(ctx, pointer);
            }
            return;
        }
        let proposal = if float_only { None } else { self.tab_proposal(&drag.panel, pointer) };
        if released {
            cancel_drag::<P>(ctx, self.id);
            if pointer.distance(drag.origin) < 2.0 || proposal.as_ref().is_some_and(|proposal| proposal.noop && !outside) {
                return;
            }
            let action = if let Some(proposal) = proposal.filter(|_| !outside) {
                Some(Action::Move { panel: drag.panel.clone(), anchor: proposal.anchor, placement: proposal.placement })
            } else if let Some(rect) = floating_rect {
                Some(Action::Float { panel: drag.panel.clone(), rect: [rect.left(), rect.top(), rect.width(), rect.height()] })
            } else {
                None
            };
            if let Some(action) = action {
                // Validate the proposed gesture against the same complete snapshot. The app
                // still applies it via its own command path after body rendering finishes.
                let mut candidate = self.layout.clone();
                if candidate.apply_with_permissions(action.clone(), |p| self.permissions(p)).is_ok() {
                    self.output.actions.push(action);
                    self.output.focused = Some(drag.panel);
                }
            }
            return;
        }
        let painter = self.preview_painter(ctx);
        if !float_only {
            self.paint_tab_proposal(ctx, &painter, &drag, proposal.as_ref(), pointer);
        }
        if let (None, Some(rect)) = (&proposal, floating_rect) {
            drop_preview(&painter, rect, self.style.accent, false);
        }
    }

    /// The group target a single tab drag would take at `pointer`.
    pub(super) fn tab_proposal(&self, panel: &P, pointer: Pos2) -> Option<DropProposal<P>> {
        let action = |to| tab_action(panel, to);
        match self.extra_proposal(std::slice::from_ref(panel), pointer, false) {
            Some(proposal) => Some(self.settled(proposal, action)),
            None => self.resolve_roomy(panel, pointer, &self.targets, action),
        }
    }

    /// The preview of a tab drag's group target (with
    /// [`crate::docking::DockStyle::side_edge_zones`], the zones of the group under the pointer),
    /// and the undock hint.
    pub(super) fn paint_tab_proposal(
        &self,
        ctx: &Context,
        painter: &egui::Painter,
        drag: &Drag<P>,
        proposal: Option<&DropProposal<P>>,
        pointer: Pos2,
    ) {
        let zones = self.paint_tab_zones(painter, &drag.panel, pointer, proposal);
        let Some(proposal) = proposal else { return };
        // Reordering inside the source strip shows only the insertion line.
        let same_group = proposal.insertion.is_some()
            && self.targets.iter().any(|target| target.rect == proposal.preview && target.tabs.iter().any(|(panel, _)| panel == &drag.panel));
        if !same_group && !proposal.noop && !zones {
            self.paint_preview(ctx, painter, proposal, true, std::slice::from_ref(&drag.panel), |to| tab_action(&drag.panel, to));
        } else {
            self.paint_marker(painter, proposal);
        }
        if !proposal.noop {
            self.paint_float_hint(ctx, pointer);
        }
    }

    /// The preview of a group target, for tab and group drags of `members`, which `action` moves
    /// there: for a drop beside the group, the rect the members then take
    /// ([`Self::result_rect`]); on the layout's outer edge band, the line along that edge; else
    /// the tab outline of the whole group, and the insertion marker on a strip. With
    /// [`crate::docking::DockStyle::side_edge_zones`], the region the drop takes, filled for a
    /// tab drag (`filled`) or a split.
    fn paint_preview(
        &self,
        ctx: &Context,
        painter: &egui::Painter,
        proposal: &DropProposal<P>,
        filled: bool,
        members: &[P],
        action: impl Fn(Target<P>) -> Action<P>,
    ) {
        if crate::docking::targets::usable(proposal.preview) {
            let beside = matches!(proposal.placement, Placement::Split(zone) | Placement::SplitSized { zone, .. } if zone != Zone::Center);
            if self.style.side_edge_zones {
                drop_preview(painter, proposal.preview.shrink(1.0), self.style.accent, filled);
            } else if matches!(proposal.placement, Placement::Edge(_)) {
                self.paint_drop_line(ctx, proposal.preview, true);
            } else if beside {
                let to = Target::At { anchor: proposal.anchor.clone(), placement: proposal.placement.clone() };
                match self.result_rect(action(to), members) {
                    Some(rect) => tab_outline(painter, rect.shrink(1.0), self.style.accent),
                    None => self.paint_drop_line(ctx, proposal.preview, false),
                }
            } else {
                tab_outline(painter, proposal.preview.shrink(1.0), self.style.accent);
            }
        }
        self.paint_marker(painter, proposal);
    }

    /// The preview of a dock drop that `action` makes of `members`: the rect they then take
    /// ([`Self::result_rect`]). A drop on the window edge ([`DockDrop::outer`]) or into an
    /// iconic column, and any dock drop with [`crate::docking::DockStyle::side_edge_zones`],
    /// shows the drop's line.
    fn paint_dock_preview(&self, ctx: &Context, painter: &egui::Painter, action: Action<P>, members: &[P], drop: &DockDrop<P>) {
        let rect = (!self.style.side_edge_zones && !drop.outer).then_some(action).and_then(|action| self.result_rect(action, members));
        match rect {
            Some(rect) => tab_outline(painter, rect.shrink(1.0), self.style.accent),
            None => self.paint_drop_line(ctx, drop.line, drop.outer),
        }
    }

    /// Where `members` are drawn once `action` is applied, laid out by the code that draws the
    /// layout: the union of the rects of the groups that hold them, or the rect of the dock
    /// column they make up alone. A floating window's tree is laid out in its body as drawn
    /// this frame. In a dock column that scrolls, a group's rect sits at the column's scroll
    /// offset this frame and is clipped to the column's body. `None` when the model rejects `action`, or when the members join an iconic column,
    /// whose icons show no rect.
    pub(super) fn result_rect(&self, action: Action<P>, members: &[P]) -> Option<Rect> {
        let mut candidate = self.layout.clone();
        candidate.apply_with_permissions(action, |p| self.permissions(p)).ok()?;
        let moved = |panels: &[P]| panels.iter().any(|p| members.contains(p));
        let sizer = self.sizer();
        let mut rects = Vec::new();
        for (root, area) in self.drawn_trees(&candidate, members) {
            let mut drawn = Vec::new();
            sizer.groups(root, area, &mut drawn);
            for Drawn { node, rect, .. } in drawn {
                let Node::Dock { side, columns } = node else {
                    if members.iter().any(|p| holds(node, p)) {
                        rects.push(rect);
                    }
                    continue;
                };
                for (index, _, rect) in self.dock_columns(*side, columns, rect).rects {
                    let Some(column) = columns.get(index).filter(|c| c.groups.iter().any(|g| moved(&g.panels))) else { continue };
                    if column.groups.iter().flat_map(|g| &g.panels).all(|p| members.contains(p)) {
                        rects.push(rect);
                        continue;
                    }
                    if column.iconic {
                        return None;
                    }
                    let (_, body, heights) = self.column_layout(column, rect);
                    // The column keeps the scroll state of its first panel, as drawn.
                    let overflow = (heights.iter().sum::<f32>() - body.height()).max(0.0);
                    let scroll = column.groups.first().and_then(|g| g.panels.first()).and_then(|p| self.column_scroll.get(p));
                    let area = body.translate(vec2(0.0, -scroll.copied().unwrap_or(0.0).min(overflow)));
                    let groups = column.groups.iter().zip(super::dock::stacked(area, &heights));
                    rects.extend(groups.filter(|(g, _)| moved(&g.panels)).map(|(_, rect)| rect.intersect(body)));
                }
            }
        }
        rects.into_iter().filter(|r| crate::docking::targets::usable(*r)).reduce(Rect::union)
    }

    /// [`Self::trees`] of `layout`, where a floating window that holds a panel other than
    /// `members` is laid out in the body that window is drawn in this frame.
    fn drawn_trees<'l>(&self, layout: &'l Layout<P>, members: &[P]) -> Vec<(&'l Node<P>, Rect)> {
        let body = |window: &crate::docking::Floating<P>| {
            let stays = |p: &&P| !members.contains(*p);
            let hit = self.frame.windows.iter().find(|hit| window.panels().filter(stays).any(|p| hit.members.contains(p)));
            hit.map_or_else(|| self.window_body(window), |hit| hit.body)
        };
        let windows = layout.floating.iter().filter(|w| !w.minimized).map(|w| (&w.root, body(w)));
        layout.root.iter().map(|root| (root, self.workspace)).chain(windows).collect()
    }

    /// The insertion marker of a drop on a tab strip.
    fn paint_marker(&self, painter: &egui::Painter, proposal: &DropProposal<P>) {
        if let Some(marker) = proposal.insertion {
            painter.rect_filled(marker, 0.0, self.style.accent);
        }
    }

    /// [`Self::paint_zones`] for a tab drag of `panel`. The center of the group holding the panel
    /// is not a target: a drop there reorders or changes nothing.
    pub(super) fn paint_tab_zones(&self, painter: &egui::Painter, panel: &P, pointer: Pos2, proposal: Option<&DropProposal<P>>) -> bool {
        if !self.style.side_edge_zones || self.extra_proposal(std::slice::from_ref(panel), pointer, false).is_some() {
            return false;
        }
        let Some(target) = self.targets.iter().rev().find(|t| crate::docking::targets::usable(t.rect) && t.rect.contains(pointer)) else {
            return false;
        };
        let own = target.tabs.iter().any(|(p, _)| p == panel);
        let at = |at: Pos2| {
            let move_to = |to| tab_action(panel, to);
            let proposal = self.resolve_roomy(panel, at, std::slice::from_ref(target), move_to)?;
            let to = Target::At { anchor: proposal.anchor.clone(), placement: proposal.placement.clone() };
            let joins = matches!(proposal.placement, Placement::Tab { .. } | Placement::Split(Zone::Center));
            (!(own && joins) && self.layout.clone().apply_with_permissions(move_to(to), |p| self.permissions(p)).is_ok()).then_some(proposal)
        };
        self.paint_zones(painter, target, proposal, at)
    }

    /// [`Self::paint_zones`] for a group drag.
    pub(super) fn paint_group_zones(&self, painter: &egui::Painter, drag: &GroupDrag<P>, pointer: Pos2, proposal: Option<&DropProposal<P>>) -> bool {
        if !self.style.side_edge_zones || self.extra_proposal(&drag.members, pointer, self.looks_through(drag)).is_some() {
            return false;
        }
        let Some(target) = self.group_target(drag, pointer) else { return false };
        self.paint_zones(painter, target, proposal, |at| self.group_proposal_at(drag, at, target))
    }

    /// With [`crate::docking::DockStyle::side_edge_zones`], a drag over a group outlines each
    /// of its drop zones that a drop would change something in (a side's trapezoid, the center
    /// rectangle, the tab strip as part of the center) and fills the zone of `proposal`, the
    /// drop under the pointer. `at` is the drop a point of `target` would get. False when it
    /// painted nothing.
    fn paint_zones(
        &self,
        painter: &egui::Painter,
        target: &DropTarget<P>,
        proposal: Option<&DropProposal<P>>,
        at: impl Fn(Pos2) -> Option<DropProposal<P>>,
    ) -> bool {
        use crate::docking::targets::{header_height, usable, zone_shape};
        let zone_of = |proposal: &DropProposal<P>| match proposal.placement {
            _ if proposal.noop => None,
            Placement::Split(zone) | Placement::SplitSized { zone, .. } => Some(zone),
            Placement::Tab { .. } => Some(Zone::Center),
            _ => None,
        };
        let header = header_height(target);
        let centroid = |points: &[Pos2]| points.iter().fold(Vec2::ZERO, |sum, p| sum + p.to_vec2()) / points.len() as f32;
        let takes = |zone: Zone, point: Pos2| at(point).as_ref().and_then(zone_of) == Some(zone);
        let mut shapes: Vec<(Zone, Vec<Pos2>)> = [Zone::Left, Zone::Right, Zone::Top, Zone::Bottom, Zone::Center]
            .into_iter()
            .filter_map(|zone| Some((zone, zone_shape(target.rect, header, SIDE_EDGE_ZONE_DEPTH, zone)?)))
            .filter(|(zone, points)| takes(*zone, centroid(points).to_pos2()))
            .collect();
        let strip = target.tab_strip.map(|strip| strip.intersect(target.rect)).filter(|strip| usable(*strip) && takes(Zone::Center, strip.center()));
        if let Some(strip) = strip {
            shapes.push((Zone::Center, vec![strip.left_top(), strip.right_top(), strip.right_bottom(), strip.left_bottom()]));
        }
        if shapes.is_empty() {
            return false;
        }
        let hovered = proposal.and_then(zone_of);
        let fill = self.style.accent.gamma_multiply(0.3);
        for (_, points) in shapes.iter().filter(|(zone, _)| Some(*zone) == hovered) {
            painter.add(egui::Shape::convex_polygon(points.clone(), fill, egui::Stroke::NONE));
        }
        let stroke = egui::Stroke::new(1.0, self.style.accent.gamma_multiply(0.6));
        for (_, points) in shapes {
            painter.add(egui::Shape::closed_line(points, stroke));
        }
        true
    }

    /// The single drop preview layer, above every floating window.
    pub(super) fn preview_painter(&self, ctx: &Context) -> egui::Painter {
        let layer = egui::LayerId::new(egui::Order::Tooltip, self.id.with("drop-preview"));
        ctx.move_to_top(layer);
        ctx.layer_painter(layer)
    }

    /// Size of a panel floated by a drag or the Float menu: the host's floating size, else the
    /// source size within the panel limits. Out of a dock or frame, or with a floating size, the
    /// height fits the content the panel last drew, up to that height (else 320). Content that
    /// filled its body floats at that height.
    pub(super) fn float_size(&self, panel: &P, source: Vec2) -> Vec2 {
        let chrome = self.style.title_height + self.style.tab_height;
        if let Some(size) = self.content.floating_size(panel).filter(|s| s.is_finite() && s.x > 0.0 && s.y > 0.0) {
            return vec2(size.x, self.fitted_height(panel, chrome, size.y).unwrap_or(size.y));
        }
        let mut size = self.iconic.float_width(panel).map_or(source, |width| vec2(width, DOCKED_FLOAT_HEIGHT));
        if matches!(self.place(panel), Some(Place::Dock { .. } | Place::Node(_))) {
            size.y = match self.fitted_height(panel, chrome, DOCKED_FLOAT_HEIGHT) {
                Some(height) => height,
                None if self.content_height(panel) == Some(None) => DOCKED_FLOAT_HEIGHT,
                None => size.y.min(DOCKED_FLOAT_HEIGHT),
            };
        }
        let limits = self.limits(panel);
        let header = vec2(0.0, self.style.tab_height);
        size.max(limits.min + header).min(limits.max + header)
    }

    /// Height of a window around `panel`'s group: `chrome` (title bar and tab strip) and the
    /// content height the panel last drew, at least [`MIN_FITTED_HEIGHT`] and at most `max`.
    /// `None` for a panel the area has not drawn.
    pub(super) fn fitted_height(&self, panel: &P, chrome: f32, max: f32) -> Option<f32> {
        let content = self.content_height(panel).flatten()?;
        Some((chrome + content).max(MIN_FITTED_HEIGHT).min(max))
    }

    /// Record what a drag floats from `panels`, one group drawn at `group` whose panels drag
    /// from `panel` rects, for commands that float without a rect.
    pub(super) fn record_float(&mut self, panels: &[P], panel: Vec2, group: Vec2) {
        let group = self.group_float_size(panels, group);
        for p in panels {
            let size = self.float_size(p, panel);
            self.float_space.panels.push((p.clone(), size));
            self.float_space.groups.push((p.clone(), group));
        }
    }

    /// Record what a drag floats from a dock column drawn at `rect`.
    pub(super) fn record_column_float(&mut self, members: Vec<P>, rect: Rect) {
        let size = self.group_float_size(&members, rect.size());
        self.float_space.columns.extend(members.into_iter().map(|p| (p, size)));
    }

    /// Size of a group or column floated by a drag or Float Tab Group: the source width, and the
    /// tallest member's floating height, else the source height capped like a panel leaving a
    /// dock column. A group fits that height to its active panel's content. A group that already
    /// floats keeps its size.
    pub(super) fn group_float_size(&self, members: &[P], source: Vec2) -> Vec2 {
        let Some(place) = members.first().and_then(|panel| self.place(panel)) else { return source };
        if matches!(place, Place::Floating { .. }) {
            return source;
        }
        let valid = |s: &Vec2| s.is_finite() && s.x > 0.0 && s.y > 0.0;
        let host = members.iter().filter_map(|panel| self.content.floating_size(panel).filter(valid)).map(|s| s.y).reduce(f32::max);
        let docked = matches!(place, Place::Dock { .. });
        let chrome = self.style.title_height + self.style.tab_height;
        let max = host.or(docked.then_some(DOCKED_FLOAT_HEIGHT));
        let fitted = max.and_then(|max| self.group_active(members, place).and_then(|active| self.fitted_height(active, chrome, max)));
        vec2(source.x, fitted.or(host).unwrap_or(if docked { source.y.min(DOCKED_FLOAT_HEIGHT) } else { source.y }))
    }

    /// The active panel of the group at `place` when `members` are that whole group.
    fn group_active<'a>(&'a self, members: &[P], place: &Place) -> Option<&'a P> {
        let (panels, active) = match place {
            Place::Node(_) | Place::Floating { .. } => match self.layout.node_of(place) {
                Some(Node::Tabs { panels, active }) => (panels, *active),
                _ => return None,
            },
            _ => self.layout.group(place).map(|group| (&group.panels, group.active))?,
        };
        (panels.len() == members.len() && members.iter().all(|m| panels.contains(m))).then(|| panels.get(active)).flatten()
    }

    /// Where a group or column drag floats with the pointer at `pointer`, keeping the grab
    /// offset, inside the window (Z2).
    fn group_ghost(&self, drag: &GroupDrag<P>, pointer: Pos2) -> Rect {
        let size = if drag.gesture == GroupGesture::Window { drag.source.size() } else { self.group_float_size(&drag.members, drag.source.size()) };
        let grab = (drag.origin - drag.source.min).clamp(Vec2::ZERO, size);
        clamp_float(Rect::from_min_size(pointer - grab, size), self.float_bounds)
    }

    fn tab_float_rect(&self, drag: &Drag<P>, pointer: Pos2) -> Rect {
        let size = self.float_size(&drag.panel, drag.source.size());
        let grab = drag.grab.min(vec2(size.x, self.style.tab_height));
        clamp_float(Rect::from_min_size(pointer - grab, size), self.float_bounds)
    }

    /// Offer the gesture to [`crate::docking::DockContent::resolve_drag`]. True when the host
    /// owns it; its feedback replaces the renderer's preview.
    pub(super) fn host_drop(
        &mut self,
        ctx: &Context,
        panel: &P,
        pointer: Pos2,
        whole_group: bool,
        released: bool,
        floating_rect: Option<Rect>,
    ) -> bool {
        let request = DropRequest {
            panel,
            pointer,
            targets: &self.targets,
            floating_rect,
            whole_group,
            released,
            workspace: self.workspace,
            screen: self.float_bounds,
        };
        let Some(feedback) = self.content.resolve_drag(ctx, self.style, &request) else { return false };
        if !released {
            let painter = self.preview_painter(ctx);
            if let Some(rect) = feedback.preview.filter(|rect| crate::docking::targets::usable(*rect)) {
                drop_preview(&painter, rect, self.style.accent, true);
                // A window moving by its title bar fades over it, as over the renderer's own.
                ctx.data_mut(|d| d.insert_temp(self.id.with(GROUP_HIGHLIGHTED), true));
            }
            if let Some(rect) = feedback.insertion.filter(|rect| crate::docking::targets::usable(*rect)) {
                painter.rect_filled(rect, 0.0, self.style.accent);
            }
        }
        true
    }
}

/// The move a single tab's drag makes.
fn tab_action<P: Clone>(panel: &P, to: Target<P>) -> Action<P> {
    match to {
        Target::At { anchor, placement } => Action::Move { panel: panel.clone(), anchor, placement },
        to => Action::MovePanel { panel: panel.clone(), to },
    }
}

/// Whether the drawn group `node` holds `panel`.
fn holds<P: PartialEq>(node: &Node<P>, panel: &P) -> bool {
    match node {
        Node::Tabs { panels, .. } => panels.contains(panel),
        Node::Stack { entries } => entries.iter().any(|e| &e.panel == panel),
        Node::Split { .. } | Node::Dock { .. } => false,
    }
}

/// The panel that names a drawn group: its first panel; none for a dock or a split.
fn lead<P>(node: &Node<P>) -> Option<&P> {
    match node {
        Node::Tabs { panels, .. } => panels.first(),
        Node::Stack { entries } => entries.first().map(|e| &e.panel),
        Node::Split { .. } | Node::Dock { .. } => None,
    }
}
