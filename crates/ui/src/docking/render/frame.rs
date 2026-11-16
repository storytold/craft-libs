//! What the split tree adds around the groups: Maximize Frame, the outer edge band, hidden
//! panels, the float hint and the group menu tail.

use std::{borrow::Cow, collections::HashMap, fmt::Debug, hash::Hash};

use egui::{Context, Pos2, Rect, pos2, vec2};

use super::{GroupKind, Renderer, floating::WindowHit, group::TabGroup};
use crate::docking::{
    Action, Column, DockContent, DockEvent, Floating, Group, HidePanels, Layout, Node, Placement, Target, Zone,
    dock_model::{Place, node_at, tree_panels},
};

/// How far below the pointer the float hint sits, clear of the cursor.
const HINT_OFFSET: f32 = 28.0;

/// What the renderer draws when it is not the layout itself, and the targets that depend on it.
pub(in crate::docking) struct FrameState<P> {
    /// View path to layout path of each split, when the view is not the layout.
    paths: Option<HashMap<Vec<bool>, Vec<bool>>>,
    /// The same for each floating window's splits, by the window's first shown panel.
    window_paths: HashMap<P, HashMap<Vec<bool>, Vec<bool>>>,
    /// The outer edge areas of top, bottom, left and right drops; none while a frame is
    /// maximized.
    edges: Option<[EdgeArea<P>; 4]>,
    /// The panels the view leaves out.
    hidden: Vec<P>,
    pub(super) maximized: Option<P>,
    /// Floating windows as drawn, back to front.
    pub(super) windows: Vec<WindowHit<P>>,
}

/// The view node an outer edge drop wraps ([`Layout::edge_path`]), and its rect once drawn.
struct EdgeArea<P> {
    path: Vec<bool>,
    /// The node is narrower than the part between the docks: the canvas column.
    narrowed: bool,
    panels: Vec<P>,
    rect: Option<Rect>,
}

impl<P: Clone + Eq + Hash> EdgeArea<P> {
    fn of(view: &Layout<P>, zone: Zone, canvas: &dyn Fn(&P) -> bool) -> Self {
        let path = view.edge_path(zone, canvas);
        let panels =
            view.root.as_ref().and_then(|root| node_at(root, &path)).map_or(Vec::new(), |node| tree_panels(node).into_iter().cloned().collect());
        Self { narrowed: view.edge_narrows(zone, canvas), path, panels, rect: None }
    }
}

const EDGES: [Zone; 4] = [Zone::Top, Zone::Bottom, Zone::Left, Zone::Right];

fn edge_areas<P: Clone + Eq + Hash>(view: &Layout<P>, canvas: &dyn Fn(&P) -> bool) -> [EdgeArea<P>; 4] {
    EDGES.map(|zone| EdgeArea::of(view, zone, canvas))
}

/// The layout as shown: a maximized frame alone, and hidden panels left out. `narrow` narrows
/// the outer edge area to the canvas column ([`Layout::edge_path`]).
pub(in crate::docking) fn view<'a, P: Clone + Eq + Hash>(
    layout: &'a Layout<P>,
    maximized: Option<&P>,
    hide: HidePanels<'_, P>,
    content: &dyn DockContent<P>,
    narrow: bool,
) -> (Cow<'a, Layout<P>>, FrameState<P>) {
    let protected = |p: &P| content.permissions(p).is_protected();
    // The edge area narrows to the canvas column without side edge zones.
    let canvas = |p: &P| narrow && protected(p);
    let hidden = |p: &P| match hide {
        HidePanels::None => false,
        HidePanels::All => !protected(p),
        HidePanels::Except(keep) => !protected(p) && !keep(p),
    };
    // Only a docked panel maximizes its frame.
    let maximized = maximized.and_then(|panel| match layout.place(panel)? {
        Place::Node(path) => Some((panel.clone(), path.clone(), layout.root.as_ref().and_then(|root| node_at(root, &path))?.clone())),
        place @ Place::Dock { .. } => {
            let group = layout.group(&place)?;
            Some((panel.clone(), path_of(&place), Node::Tabs { panels: group.panels.clone(), active: group.active }))
        }
        Place::Floating { .. } => None,
    });
    let mut state = FrameState { paths: None, window_paths: HashMap::new(), edges: None, hidden: Vec::new(), maximized: None, windows: Vec::new() };
    if maximized.is_none() && matches!(hide, HidePanels::None) {
        state.edges = Some(edge_areas(layout, &canvas));
        return (Cow::Borrowed(layout), state);
    }
    state.hidden = layout.panels().into_iter().filter(|p| hidden(p)).cloned().collect();
    let mut paths = HashMap::new();
    let view = match maximized {
        Some((panel, path, node)) => {
            state.maximized = Some(panel);
            Layout { root: prune(&node, path, Vec::new(), &hidden, &mut paths), floating: Vec::new() }
        }
        None => {
            let root = layout.root.as_ref().and_then(|root| prune(root, Vec::new(), Vec::new(), &hidden, &mut paths));
            let mut floating = Vec::new();
            for window in &layout.floating {
                let mut paths = HashMap::new();
                let Some(root) = prune(&window.root, Vec::new(), Vec::new(), &hidden, &mut paths) else { continue };
                let shown = Floating { root, ..window.clone() };
                if let Some(key) = shown.panels().next() {
                    state.window_paths.insert(key.clone(), paths);
                }
                floating.push(shown);
            }
            Layout { root, floating }
        }
    };
    if state.maximized.is_none() {
        state.edges = Some(edge_areas(&view, &canvas));
    }
    state.paths = Some(paths);
    (Cow::Owned(view), state)
}

fn path_of(place: &Place) -> Vec<bool> {
    match place {
        Place::Node(path) | Place::Dock { path, .. } => path.clone(),
        Place::Floating { .. } => Vec::new(),
    }
}

fn visible<P>(node: &Node<P>, hidden: &dyn Fn(&P) -> bool) -> bool {
    match node {
        Node::Split { first, second, .. } => visible(first, hidden) || visible(second, hidden),
        Node::Tabs { panels, .. } => panels.iter().any(|p| !hidden(p)),
        Node::Stack { entries } => entries.iter().any(|e| !hidden(&e.panel)),
        Node::Dock { columns, .. } => columns.iter().flat_map(|c| &c.groups).flat_map(|g| &g.panels).any(|p| !hidden(p)),
    }
}

/// `node` without hidden panels; a split with one visible side becomes that side. Records the
/// layout path of every split kept.
fn prune<P: Clone + PartialEq>(
    node: &Node<P>,
    path: Vec<bool>,
    at: Vec<bool>,
    hidden: &dyn Fn(&P) -> bool,
    paths: &mut HashMap<Vec<bool>, Vec<bool>>,
) -> Option<Node<P>> {
    let child = |path: &[bool], side: bool| [path, &[side]].concat();
    match node {
        Node::Split { axis, size, first, second } => match (visible(first, hidden), visible(second, hidden)) {
            (true, true) => {
                let first = prune(first, child(&path, false), child(&at, false), hidden, paths)?;
                let second = prune(second, child(&path, true), child(&at, true), hidden, paths)?;
                paths.insert(at, path);
                Some(Node::Split { axis: *axis, size: *size, first: Box::new(first), second: Box::new(second) })
            }
            (true, false) => prune(first, child(&path, false), at, hidden, paths),
            (false, true) => prune(second, child(&path, true), at, hidden, paths),
            (false, false) => None,
        },
        Node::Tabs { panels, active } => {
            let group = prune_group(&Group::new(panels.clone(), *active), hidden)?;
            Some(Node::Tabs { panels: group.panels, active: group.active })
        }
        Node::Stack { entries } => {
            let entries: Vec<_> = entries.iter().filter(|e| !hidden(&e.panel)).cloned().collect();
            (!entries.is_empty()).then_some(Node::Stack { entries })
        }
        Node::Dock { side, columns } => {
            let columns: Vec<_> = columns
                .iter()
                .filter_map(|c| {
                    let groups: Vec<_> = c.groups.iter().filter_map(|g| prune_group(g, hidden)).collect();
                    (!groups.is_empty()).then(|| Column { groups, ..c.clone() })
                })
                .collect();
            (!columns.is_empty()).then_some(Node::Dock { side: *side, columns })
        }
    }
}

/// A group without hidden panels; the selected panel stays selected when it is shown.
fn prune_group<P: Clone + PartialEq>(group: &Group<P>, hidden: &dyn Fn(&P) -> bool) -> Option<Group<P>> {
    let selected = group.active_panel();
    let panels: Vec<P> = group.panels.iter().filter(|p| !hidden(p)).cloned().collect();
    let active = selected.and_then(|s| panels.iter().position(|p| p == s)).unwrap_or(0);
    (!panels.is_empty()).then(|| Group { panels, active, ..group.clone() })
}

impl<P: Clone + Eq + Hash + Debug + Send + Sync + 'static> Renderer<'_, P> {
    pub(super) fn note_edge(&mut self, path: &[bool], rect: Rect) {
        if self.tree.is_some() {
            return;
        }
        for area in self.frame.edges.iter_mut().flatten().filter(|area| area.path == path) {
            area.rect = Some(rect);
        }
    }

    /// The layout path of a split drawn at view path `path`, in the tree being drawn.
    pub(super) fn original_path(&self, path: &[bool]) -> Vec<bool> {
        let paths = match &self.tree {
            None => self.frame.paths.as_ref(),
            Some((key, _)) => self.frame.window_paths.get(key),
        };
        paths.and_then(|paths| paths.get(path).cloned()).unwrap_or_else(|| path.to_vec())
    }

    /// The outer edge band under the pointer (P8): the top and bottom of
    /// [`crate::docking::DockStyle::drop_edges`], all four with
    /// [`crate::docking::DockStyle::side_edge_zones`]. Returns a docked anchor that is not
    /// moving, the zone and the preview: a line along the edge area's whole edge, or with side
    /// edge zones the quarter the new frame takes. Without them, on a tab strip that takes tabs,
    /// the band reaches only [`crate::docking::targets::STRIP_REACH`] from the line.
    pub(super) fn edge_proposal(&self, members: &[P], pointer: Pos2) -> Option<(P, Zone, Rect)> {
        let sides = self.style.outer_edges();
        let (distance, zone, area) = EDGES
            .into_iter()
            .filter(|zone| sides.contains(*zone))
            .filter_map(|zone| {
                let area = self.edge_area(zone)?.rect.filter(|area| area.contains(pointer))?;
                let distance = match zone {
                    Zone::Top => pointer.y - area.top(),
                    Zone::Bottom => area.bottom() - pointer.y,
                    Zone::Left => pointer.x - area.left(),
                    Zone::Right | Zone::Center => area.right() - pointer.x,
                };
                Some((distance, zone, area))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))?;
        let band = if self.style.side_edge_zones { crate::docking::targets::SIDE_EDGE_BAND } else { self.style.edge_band };
        if distance > band.min(area.width() / 4.0).min(area.height() / 4.0) {
            return None;
        }
        let line = crate::docking::targets::edge_line(area, zone, self.style.drop_line_width);
        let reach = |line: Rect| line.distance_to_pos(pointer) <= crate::docking::targets::STRIP_REACH;
        if !self.style.side_edge_zones && self.in_tab_strip(pointer) && !line.is_some_and(reach) {
            return None;
        }
        let anchor = self.edge_anchor(members, zone)?;
        if !self.style.side_edge_zones {
            return Some((anchor, zone, line?));
        }
        let size = area.size() * 0.25;
        let preview = match zone {
            Zone::Left => Rect::from_min_size(area.min, vec2(size.x, area.height())),
            Zone::Right => Rect::from_min_max(pos2(area.right() - size.x, area.top()), area.max),
            Zone::Top => Rect::from_min_size(area.min, vec2(area.width(), size.y)),
            Zone::Bottom | Zone::Center => Rect::from_min_max(pos2(area.left(), area.bottom() - size.y), area.max),
        };
        Some((anchor, zone, preview))
    }

    fn edge_area(&self, zone: Zone) -> Option<&EdgeArea<P>> {
        self.frame.edges.as_ref()?.get(EDGES.iter().position(|edge| *edge == zone)?)
    }

    /// The docked anchor, not one of `members`, of an outer edge drop at `zone` that wraps the
    /// edge area drawn: protected where that area is the canvas column, as the model narrows on
    /// a protected anchor, else not. The drawn area and the model's can still differ (hidden
    /// panels, a canvas test that differs from these permissions), so the drop is tried: an
    /// anchor whose drop wraps other panels than the area shows gives way to one that is not
    /// protected, and then to no edge target.
    fn edge_anchor(&self, members: &[P], zone: Zone) -> Option<P> {
        let area = self.edge_area(zone)?;
        let protected = |p: &P| self.permissions(p).is_protected();
        let skip = |p: &P| members.contains(p);
        let lead = members.first()?;
        let wraps_area = |anchor: &P| {
            let to = Target::At { anchor: anchor.clone(), placement: Placement::Edge(zone) };
            let action = match members {
                [_] => Action::Move { panel: lead.clone(), anchor: anchor.clone(), placement: Placement::Edge(zone) },
                _ => Action::MoveGroup { panel: lead.clone(), to },
            };
            let mut candidate = self.layout.clone();
            if candidate.apply_with_permissions(action, |p| self.permissions(p)).is_err() {
                // Left to the model's check.
                return true;
            }
            let Some(Place::Node(path)) = candidate.place(lead) else { return false };
            let Some((last, parent)) = path.split_last() else { return false };
            let beside = [parent, &[!last]].concat();
            let Some(wrapped) = candidate.root.as_ref().and_then(|root| node_at(root, &beside)) else { return false };
            let shown = |p: &&P| !self.frame.hidden.contains(p) && !members.contains(p);
            let wrapped: Vec<&P> = tree_panels(wrapped).into_iter().filter(shown).collect();
            let drawn: Vec<&P> = area.panels.iter().filter(shown).collect();
            wrapped.len() == drawn.len() && wrapped.iter().all(|p| drawn.contains(p))
        };
        let anchor = self.layout.edge_anchor(area.narrowed, &protected, &skip)?;
        if wraps_area(&anchor) {
            return Some(anchor);
        }
        self.layout.edge_anchor(false, &protected, &skip).filter(|other| *other != anchor && !protected(other) && wraps_area(other))
    }

    /// Under the pointer while a drag offers a docking target: which modifier floats the drop
    /// instead. With native windows, the window under the pointer draws it, beside its preview.
    pub(super) fn paint_float_hint(&self, ctx: &Context, pointer: Pos2) {
        if !self.style.float_hint || self.style.float_hint_label.is_empty() {
            return;
        }
        let key = if ctx.os() == egui::os::OperatingSystem::Mac { "⌘" } else { "Ctrl" };
        let painter = self.preview_painter(ctx);
        let galley = painter.layout_no_wrap(self.style.float_hint_label.replace("{key}", key), self.style.font.clone(), self.style.text);
        let rect = Rect::from_center_size(pointer + vec2(0.0, HINT_OFFSET), galley.size() + vec2(12.0, 6.0));
        let rect = crate::docking::chrome::clamp(rect, self.float_bounds);
        painter.rect(rect, 4.0, self.style.floating_background, egui::Stroke::new(1.0, self.style.accent), egui::StrokeKind::Inside);
        painter.galley(rect.center() - galley.size() * 0.5, galley, self.style.text);
    }

    /// Ctrl/Cmd held when a split-tree tab drag starts: the drop floats (S9).
    pub(super) fn latch_undock(&self, ctx: &Context, kind: Option<GroupKind>) {
        let command = ctx.input(|i| i.modifiers.command || i.modifiers.ctrl || i.modifiers.mac_cmd);
        if self.style.command_press_floats && matches!(kind, Some(GroupKind::Frame | GroupKind::Floating { .. })) && command {
            ctx.data_mut(|d| {
                if let Some(drag) = d.get_temp::<crate::docking::Drag<P>>(self.id.with("drag")) {
                    d.insert_temp(self.id.with("drag"), crate::docking::Drag { undock: true, ..drag });
                }
            });
        }
    }

    /// The menu tail: Close Other Panels in Group and Maximize Frame when the style shows
    /// them.
    pub(super) fn frame_items(&mut self, ui: &mut egui::Ui, owner: &TabGroup<'_, P>, panel: &P) {
        let others = owner.panels.iter().filter(|p| *p != panel).all(|p| self.permissions(p).closable);
        if self.style.close_others_item
            && owner.panels.len() > 1
            && others
            && self.style.close_menu_items
            && self.menu_action(ui, &self.style.close_others_label.clone())
        {
            self.output.actions.push(Action::CloseOthers { panel: panel.clone() });
        }
        if self.style.maximize_item && owner.kind == GroupKind::Frame {
            let restore = self.frame.maximized.as_ref().is_some_and(|m| owner.panels.contains(m));
            let label = if restore { self.style.restore_frame_label.clone() } else { self.style.maximize_label.clone() };
            if self.menu_action(ui, &label) {
                let panel = if restore { self.frame.maximized.clone().unwrap_or_else(|| panel.clone()) } else { panel.clone() };
                self.output.events.push(DockEvent::ToggleMaximize { panel });
            }
        }
    }
}
