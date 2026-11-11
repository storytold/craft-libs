//! What the tree adds around the groups: Maximize Frame, the hidden-panel prune and the outer edge bands.

use super::config::DockConfig;
use super::event::HidePanels;
use super::targets::{DockSizes, DockSpot, DockTargets};
use super::tree;
use craft_behavior::DockSide;
use craft_behavior::dock::{Column, Floating, Group, Layout, Node, PanelPlace, SplitSize, node_at};
use egui::{Context, Id, Pos2, Rect, Vec2};
use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::Hash;

/// A layout as it is shown: a maximized frame alone, hidden panels left out.
#[derive(Debug)]
#[non_exhaustive]
pub struct FrameView<'a, P: Clone> {
    /// The layout to draw.
    pub layout: Cow<'a, Layout<P>>,
    /// The panel whose frame is maximized.
    pub maximized: Option<P>,
    /// The panels the view leaves out.
    pub hidden: Vec<P>,
    /// The layout path of each split of the view, by the split's view path; `None` when the view is the layout.
    pub paths: Option<HashMap<Vec<bool>, Vec<bool>>>,
    /// The layout index of each floating window of the view, in order.
    pub windows: Vec<usize>,
    /// For each floating window of the view, the path in the window's layout tree of each split of its pruned tree.
    window_paths: Vec<HashMap<Vec<bool>, Vec<bool>>>,
}

impl<P: Clone> FrameView<'_, P> {
    /// Writes the split sizes of `edited`, an edit of the view's layout, into `layout`, the layout the view was made
    /// from. Panels the view left out keep their place; only sizes change.
    pub fn write_back(&self, edited: &Layout<P>, layout: &mut Layout<P>) {
        let Some(paths) = &self.paths else {
            layout.clone_from(edited);
            return;
        };
        if let (Some(shown), Some(root)) = (edited.root.as_ref(), layout.root.as_mut()) {
            tree::apply_sizes(root, &split_sizes(shown, paths));
        }
        for ((shown, at), paths) in edited.floating.iter().zip(&self.windows).zip(&self.window_paths) {
            if let Some(window) = layout.floating.get_mut(*at) {
                tree::apply_sizes(&mut window.root, &split_sizes(&shown.root, paths));
            }
        }
    }
}

/// The size of every split of the view's tree `node`, with the path in the full tree that `paths` maps its own to.
fn split_sizes<P>(node: &Node<P>, paths: &HashMap<Vec<bool>, Vec<bool>>) -> Vec<(Vec<bool>, SplitSize)> {
    fn walk<P>(node: &Node<P>, at: &mut Vec<bool>, paths: &HashMap<Vec<bool>, Vec<bool>>, out: &mut Vec<(Vec<bool>, SplitSize)>) {
        let Node::Split { size, first, second, .. } = node else { return };
        if let Some(path) = paths.get(at.as_slice()) {
            out.push((path.clone(), *size));
        }
        for (side, child) in [(false, first), (true, second)] {
            at.push(side);
            walk(child, at, paths, out);
            at.pop();
        }
    }
    let mut out = Vec::new();
    walk(node, &mut Vec::new(), paths, &mut out);
    out
}

/// The layout as shown: the frame of `maximized` alone when it is docked, and `hide`'s panels left out. Protected
/// panels always stay.
pub fn maximize<'a, P: Clone + Eq + Hash>(
    layout: &'a Layout<P>,
    maximized: Option<&P>,
    hide: HidePanels<'_, P>,
    protected: &dyn Fn(&P) -> bool,
) -> FrameView<'a, P> {
    let hidden = |panel: &P| match hide {
        HidePanels::None => false,
        HidePanels::All => !protected(panel),
        HidePanels::Except(keep) => !protected(panel) && !keep(panel),
    };
    // Only a docked panel maximizes its frame.
    let frame = maximized.and_then(|panel| match layout.place(panel)? {
        place @ PanelPlace::Node(_) => {
            let node = layout.node_of(&place)?;
            Some((panel.clone(), path_of(&place), node.clone()))
        }
        place @ PanelPlace::Dock { .. } => {
            let group = layout.group(&place)?;
            Some((panel.clone(), path_of(&place), Node::tabs(group.panels.clone(), group.active)))
        }
        _ => None,
    });
    let unchanged = frame.is_none() && matches!(hide, HidePanels::None);
    if unchanged {
        return FrameView {
            layout: Cow::Borrowed(layout),
            maximized: None,
            hidden: Vec::new(),
            paths: None,
            windows: (0..layout.floating.len()).collect(),
            window_paths: Vec::new(),
        };
    }
    let left_out = layout.panels().into_iter().filter(|panel| hidden(panel)).cloned().collect();
    let mut paths = HashMap::new();
    let mut windows = Vec::new();
    let mut window_paths = Vec::new();
    let (shown, kept) = match frame {
        Some((panel, path, node)) => (Layout::new(prune(&node, path, Vec::new(), &hidden, &mut paths), Vec::new()), Some(panel)),
        None => {
            let root = layout.root.as_ref().and_then(|root| prune(root, Vec::new(), Vec::new(), &hidden, &mut paths));
            let mut floating = Vec::new();
            for (at, window) in layout.floating.iter().enumerate() {
                let mut window_map = HashMap::new();
                let Some(root) = prune(&window.root, Vec::new(), Vec::new(), &hidden, &mut window_map) else { continue };
                window_paths.push(window_map);
                let mut shown: Floating<P> = window.clone();
                shown.root = root;
                floating.push(shown);
                windows.push(at);
            }
            (Layout::new(root, floating), None)
        }
    };
    FrameView { layout: Cow::Owned(shown), maximized: kept, hidden: left_out, paths: Some(paths), windows, window_paths }
}

fn path_of(place: &PanelPlace) -> Vec<bool> {
    match place {
        PanelPlace::Node(path) | PanelPlace::Dock { path, .. } | PanelPlace::Floating { path, .. } => path.clone(),
        _ => Vec::new(),
    }
}

fn visible<P>(node: &Node<P>, hidden: &dyn Fn(&P) -> bool) -> bool {
    match node {
        Node::Split { first, second, .. } => visible(first, hidden) || visible(second, hidden),
        Node::Tabs { panels, .. } => panels.iter().any(|panel| !hidden(panel)),
        Node::Stack { entries, .. } => entries.iter().any(|entry| !hidden(&entry.panel)),
        Node::Dock { columns, .. } => columns.iter().flat_map(|column| &column.groups).flat_map(|group| &group.panels).any(|panel| !hidden(panel)),
        _ => true,
    }
}

/// `node` without hidden panels; a split with one visible side becomes that side. Records the layout path of every
/// split kept, by its path in the pruned tree.
fn prune<P: Clone + PartialEq>(
    node: &Node<P>,
    path: Vec<bool>,
    at: Vec<bool>,
    hidden: &dyn Fn(&P) -> bool,
    paths: &mut HashMap<Vec<bool>, Vec<bool>>,
) -> Option<Node<P>> {
    let child = |path: &[bool], side: bool| [path, &[side]].concat();
    match node {
        Node::Split { axis, size, first, second, .. } => match (visible(first, hidden), visible(second, hidden)) {
            (true, true) => {
                let first = prune(first, child(&path, false), child(&at, false), hidden, paths)?;
                let second = prune(second, child(&path, true), child(&at, true), hidden, paths)?;
                paths.insert(at, path);
                Some(Node::split(*axis, *size, first, second))
            }
            (true, false) => prune(first, child(&path, false), at, hidden, paths),
            (false, true) => prune(second, child(&path, true), at, hidden, paths),
            (false, false) => None,
        },
        Node::Tabs { panels, active, .. } => {
            let group = prune_group(&Group::new(panels.clone(), *active), hidden)?;
            Some(Node::tabs(group.panels, group.active))
        }
        Node::Stack { entries, .. } => {
            let entries: Vec<_> = entries.iter().filter(|entry| !hidden(&entry.panel)).cloned().collect();
            (!entries.is_empty()).then(|| Node::stack(entries))
        }
        Node::Dock { side, columns, .. } => {
            let columns: Vec<Column<P>> = columns
                .iter()
                .filter_map(|column| {
                    let groups: Vec<_> = column.groups.iter().filter_map(|group| prune_group(group, hidden)).collect();
                    (!groups.is_empty()).then(|| {
                        let mut kept = column.clone();
                        kept.groups = groups;
                        kept
                    })
                })
                .collect();
            (!columns.is_empty()).then(|| Node::dock(*side, columns))
        }
        _ => Some(node.clone()),
    }
}

/// A group without hidden panels; the selected panel stays selected when it is shown.
fn prune_group<P: Clone + PartialEq>(group: &Group<P>, hidden: &dyn Fn(&P) -> bool) -> Option<Group<P>> {
    let selected = group.active_panel();
    let panels: Vec<P> = group.panels.iter().filter(|panel| !hidden(panel)).cloned().collect();
    let active = selected.and_then(|selected| panels.iter().position(|panel| panel == selected)).unwrap_or(0);
    (!panels.is_empty()).then(|| {
        let mut kept = group.clone();
        kept.panels = panels;
        kept.active = active;
        kept
    })
}

/// One outer edge of the docked area where a dropped group wraps the whole edge area.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct EdgeBand {
    /// The side in reading order.
    pub side: DockSide,
    /// The edge area the drop wraps.
    pub area: Rect,
    /// The strip along the area's edge that takes the drop.
    pub band: Rect,
    /// The area is narrower than the part between the docks: the canvas column.
    pub narrowed: bool,
}

/// The bands of the outer edges `config.drop_edges` allows, for the leaves of a layout as drawn, each as its tree path
/// and rect. An edge area is the node `Layout::edge_path` names, with `canvas` marking the panels it narrows to. Without side zones the band is the
/// edge band, with them the side edge band; either is at most a quarter of the area.
pub fn edge_bands<P: Clone + Eq + Hash>(
    layout: &Layout<P>,
    leaves: &[(Vec<bool>, Rect)],
    canvas: &dyn Fn(&P) -> bool,
    config: &DockConfig,
    sizes: &DockSizes,
    rtl: bool,
) -> Vec<EdgeBand> {
    let Some(root) = layout.root.as_ref() else { return Vec::new() };
    [DockSide::Top, DockSide::Bottom, DockSide::Start, DockSide::End]
        .into_iter()
        .filter(|side| config.drop_edges.contains(*side))
        .filter_map(|side| {
            let path = layout.edge_path(side, canvas);
            node_at(root, &path)?;
            let area = leaves.iter().filter(|(leaf, _)| leaf.starts_with(&path)).map(|(_, rect)| *rect).reduce(|all, rect| all.union(rect))?;
            let depth = if config.side_edge_zones { sizes.side_edge_band } else { sizes.edge_band };
            let depth = depth.min(area.width() / 4.0).min(area.height() / 4.0);
            let left = matches!((side, rtl), (DockSide::Start, false) | (DockSide::End, true));
            let right = matches!((side, rtl), (DockSide::End, false) | (DockSide::Start, true));
            let band = if left {
                Rect::from_min_size(area.min, Vec2::new(depth, area.height()))
            } else if right {
                Rect::from_min_max(Pos2::new(area.right() - depth, area.top()), area.max)
            } else if side == DockSide::Top {
                Rect::from_min_size(area.min, Vec2::new(area.width(), depth))
            } else {
                Rect::from_min_max(Pos2::new(area.left(), area.bottom() - depth), area.max)
            };
            Some(EdgeBand { side, area, band, narrowed: layout.edge_narrows(side, canvas) })
        })
        .collect()
}

/// Registers the bands as the dock's edge spots, for the next pass's drops.
pub fn register_edge_bands<P: Clone + Send + 'static>(ctx: &Context, dock: Id, bands: &[EdgeBand]) {
    for band in bands {
        DockTargets::<P>::register(ctx, dock, DockSpot::Edge(band.side), band.band);
    }
}
