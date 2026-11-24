//! Where each docking gesture starts, for control protocols and tests.

use std::{collections::HashMap, fmt::Debug, hash::Hash};

use egui::{Context, Id, LayerId, Order, Rect};

use super::{DockArea, Floating, Layout, Node, render::RESIZE_HANDLES};

/// A rendered docking control that starts a gesture. Groups, columns and windows are named by
/// their first panel.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Hit<P> {
    /// A tab: click activates, drag moves the panel (S1).
    Tab(P),
    /// A group's drag bar: drag moves the group; a right click, Enter or Space opens the group
    /// menu (S2).
    DragBar(P),
    /// The top border of a group's strip: drag moves the group (S3).
    Border(P),
    /// The "…" button of a group whose tabs overflow; named by the active panel.
    Overflow(P),
    /// [D] A column bar: drag moves the column (S6).
    ColumnBar(P),
    /// [D] The »/« button of a column bar (M2).
    Collapse(P),
    /// An accordion entry's −/+ button.
    StackToggle(P),
    /// A floating window's title bar (S5).
    Title(P),
    /// A floating window's × button.
    Close(P),
    /// [D] A panel's icon in an iconic column (S7, M5).
    Icon(P),
    /// [D] The grip above an icon group (S8).
    IconGrip(P),
    /// A floating window's resize handle.
    Resize(P, ResizeEdge),
    Divider(Divider<P>),
}

/// The edge or corner a floating window's resize handle moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ResizeEdge {
    Left,
    Right,
    Bottom,
    BottomLeft,
    BottomRight,
    Top,
    TopLeft,
    TopRight,
}

/// A divider that resizes what is on either side of it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Divider<P> {
    /// A split, by its path from the root (see [`super::Action::ResizeSplit`]).
    Split(Vec<bool>),
    /// Below the accordion entry of this panel.
    Stack(P),
    /// [D] The inner edge of the column holding this panel first.
    Column(P),
    /// Below the dock group holding this panel first.
    Group(P),
    /// Where the divider of a split inside another split ends on the outer split's divider,
    /// by the outer and the inner split's paths. Dragging it moves both dividers.
    Intersection(Vec<bool>, Vec<bool>),
    /// A divider in the floating window holding the panel, by its path from the window's root.
    WindowSplit(P, Vec<bool>),
    WindowIntersection(P, Vec<bool>, Vec<bool>),
}

fn key(area: Id) -> Id {
    area.with("hit-rects")
}

impl<P: Clone + Eq + Hash + Debug + Send + Sync + 'static> DockArea<'_, P> {
    /// Every gesture source the area drew in its last frame, with its rect in points. Hosts
    /// return these from a `ui.docking` control method beside the layout and `focused`; tests
    /// drag from their centers.
    pub fn hit_rects(ctx: &Context, id: Id) -> Vec<(Hit<P>, Rect)> {
        let Some((layout, rects)) = ctx.data(|d| d.get_temp::<Recorded<P>>(key(id))) else { return Vec::new() };
        let mut names = named(id, &layout);
        rects.into_iter().filter_map(|(id, rect)| Some((names.remove(&id)?, rect))).collect()
    }

    /// The egui widget id of the control `hit` names in the area `id`, for tests and
    /// automation that focus a control or read its response.
    pub fn hit_id(id: Id, hit: &Hit<P>) -> Id {
        match hit {
            Hit::Tab(p) => id.with(("tab", p)),
            Hit::Icon(p) => id.with(("icon", p)),
            Hit::Overflow(p) => id.with(("overflow", p)),
            Hit::StackToggle(p) => id.with(("collapse", p)),
            Hit::Divider(Divider::Stack(p)) => id.with(("stack-resize", p)),
            Hit::DragBar(p) => id.with(("drag-bar", Some(p))),
            Hit::Border(p) => id.with(("drag-border", Some(p))),
            Hit::IconGrip(p) => id.with(("icon-grip", Some(p))),
            Hit::ColumnBar(p) => id.with(("column-bar", Some(p))),
            Hit::Collapse(p) => id.with(("column-collapse", Some(p))),
            Hit::Divider(Divider::Column(p)) => id.with(("column-resize", Some(p))),
            Hit::Divider(Divider::Group(p)) => id.with(("group-resize", Some(p))),
            Hit::Divider(Divider::Split(path)) => id.with(("split", path)),
            Hit::Divider(Divider::Intersection(outer, inner)) => id.with(("intersection", outer, inner)),
            Hit::Divider(Divider::WindowSplit(p, path)) => id.with(("window-split", p, path)),
            Hit::Divider(Divider::WindowIntersection(p, outer, inner)) => id.with(("window-intersection", p, outer, inner)),
            Hit::Title(p) => id.with(("move-group", Some(p))),
            Hit::Close(p) => id.with(("move-group", Some(p))).with("close"),
            Hit::Resize(p, edge) => {
                let name = match edge {
                    ResizeEdge::BottomRight => RESIZE_HANDLES[0],
                    ResizeEdge::Left => RESIZE_HANDLES[1],
                    ResizeEdge::Right => RESIZE_HANDLES[2],
                    ResizeEdge::Bottom => RESIZE_HANDLES[3],
                    ResizeEdge::BottomLeft => RESIZE_HANDLES[4],
                    ResizeEdge::Top => RESIZE_HANDLES[5],
                    ResizeEdge::TopLeft => RESIZE_HANDLES[6],
                    ResizeEdge::TopRight => RESIZE_HANDLES[7],
                };
                id.with(("floating", p)).with(name)
            }
        }
    }

    /// The egui layer of a floating window in the area `id`; `None` for an empty window. Of two
    /// layers, the later in `ctx.memory(|m| m.layer_ids())` is drawn on top, and
    /// `ctx.memory(|m| m.area_rect(layer.id))` is the window's rect. A window the area does not
    /// draw (all its panels hidden, or a frame maximized) keeps its last rect there: check
    /// `ctx.memory(|m| m.areas().is_visible(&layer))` before treating it as covering anything.
    pub fn floating_layer(id: Id, window: &Floating<P>) -> Option<LayerId> {
        window.panels().next().map(|first| LayerId::new(Order::Middle, id.with(("floating", first))))
    }
}

/// Every control the area `area` can draw for `layout`, by widget id.
pub(super) fn named<P: Clone + Eq + Hash + Debug + Send + Sync + 'static>(area: Id, layout: &Layout<P>) -> HashMap<Id, Hit<P>> {
    let mut names: HashMap<Id, Hit<P>> = HashMap::new();
    let mut name = |hit: Hit<P>| {
        names.insert(DockArea::<P>::hit_id(area, &hit), hit);
    };
    for panel in layout.panels() {
        let p = || panel.clone();
        for hit in [
            Hit::Tab(p()),
            Hit::Icon(p()),
            Hit::Overflow(p()),
            Hit::StackToggle(p()),
            Hit::Divider(Divider::Stack(p())),
            Hit::DragBar(p()),
            Hit::Border(p()),
            Hit::IconGrip(p()),
            Hit::ColumnBar(p()),
            Hit::Collapse(p()),
            Hit::Divider(Divider::Column(p())),
            Hit::Divider(Divider::Group(p())),
            Hit::Title(p()),
            Hit::Close(p()),
        ] {
            name(hit);
        }
        for edge in [
            ResizeEdge::BottomRight,
            ResizeEdge::Left,
            ResizeEdge::Right,
            ResizeEdge::Bottom,
            ResizeEdge::BottomLeft,
            ResizeEdge::Top,
            ResizeEdge::TopLeft,
            ResizeEdge::TopRight,
        ] {
            name(Hit::Resize(p(), edge));
        }
    }
    if let Some(root) = &layout.root {
        name_splits(root, &mut |path, outer| match outer {
            None => name(Hit::Divider(Divider::Split(path))),
            Some(outer) => name(Hit::Divider(Divider::Intersection(outer, path))),
        });
    }
    for window in &layout.floating {
        let Some(key) = window.panels().next() else { continue };
        name_splits(&window.root, &mut |path, outer| match outer {
            None => name(Hit::Divider(Divider::WindowSplit(key.clone(), path))),
            Some(outer) => name(Hit::Divider(Divider::WindowIntersection(key.clone(), outer, path))),
        });
    }
    names
}

/// Every split of a tree by its path (`None`), and every pair of a split and a split inside it
/// (`Some(outer)`), whose dividers may meet.
fn name_splits<P>(root: &Node<P>, name: &mut dyn FnMut(Vec<bool>, Option<Vec<bool>>)) {
    let mut nodes = vec![(root, Vec::<bool>::new())];
    let mut splits: Vec<Vec<bool>> = Vec::new();
    while let Some((node, path)) = nodes.pop() {
        if splits.len() > super::model::MAX_NODES {
            break;
        }
        if let Node::Split { first, second, .. } = node {
            name(path.clone(), None);
            for outer in splits.iter().filter(|outer| path.starts_with(outer)) {
                name(path.clone(), Some(outer.clone()));
            }
            splits.push(path.clone());
            let mut second_path = path.clone();
            second_path.push(true);
            let mut first_path = path;
            first_path.push(false);
            nodes.push((second, second_path));
            nodes.push((first, first_path));
        }
    }
}

/// The layout of the last frame and the rects of the controls it registered, by widget id.
type Recorded<P> = (Layout<P>, Vec<(Id, Rect)>);

/// Store the rects of the controls the renderer registered this frame. They are named only
/// when [`DockArea::hit_rects`] asks, which tests and automation do, not every frame.
pub(super) fn record<P: Clone + Eq + Hash + Debug + Send + Sync + 'static>(ctx: &Context, area: Id, layout: &Layout<P>, owned: &[Id]) {
    let rects: Vec<(Id, Rect)> =
        owned.iter().filter_map(|id| Some((*id, ctx.read_response(*id)?.rect))).filter(|(_, rect)| rect.is_positive()).collect();
    ctx.data_mut(|d| d.insert_temp::<Recorded<P>>(key(area), (layout.clone(), rects)));
}
