//! The docked tree, its leaves and the floating windows.

use emath::Rect;

use crate::{flow::Axis, placement::Placement};

use super::{
    groups::{Column, DockEdge, Group},
    split::SplitSize,
};

/// A node of a layout tree.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum Node<P> {
    /// Two children side by side or stacked.
    #[non_exhaustive]
    Split {
        /// Direction the children lie in.
        axis: Axis,
        /// How the two children share the room.
        size: SplitSize,
        /// The first child: the start (or top) child.
        first: Box<Node<P>>,
        /// The second child: the end (or bottom) child.
        second: Box<Node<P>>,
    },
    /// A group of tabs.
    #[non_exhaustive]
    Tabs {
        /// The tabs, first to last.
        panels: Vec<P>,
        /// Index into `panels` of the selected tab.
        active: usize,
    },
    /// An accordion keeps its open state and preferred heights when other panels move.
    #[non_exhaustive]
    Stack {
        /// The accordion sections, first to last.
        entries: Vec<StackEntry<P>>,
    },
    /// One window edge's dock, columns from the window edge inwards. A dock is the root or,
    /// following only same-side children of horizontal splits, at its window edge. At most one
    /// per side. An empty dock (no columns) is valid and accepts `Target::NewDock`.
    #[non_exhaustive]
    Dock {
        /// The window edge the dock sits on.
        side: DockEdge,
        /// The columns, in order from the window edge inward.
        columns: Vec<Column<P>>,
    },
}

/// One panel of a [`Node::Stack`] accordion.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct StackEntry<P> {
    /// The section's panel.
    pub panel: P,
    /// The section shows its body.
    pub open: bool,
    /// Preferred height in points; `None` shares the free height.
    pub height: Option<f32>,
}

/// A floating window: one column of groups, the same [`Column`] a dock holds. Nothing sits beside the column
/// inside a window; its collapsed state is the column's [`Column::iconic`].
///
/// Serde reads every saved shape (see `floating_serde`); a window that is one expanded column with one plain group
/// and no native rect writes the v1 shape `{panels, active, rect}`, so older builds can read files that use no
/// newer feature. Every window writes its `id`; a window read without one gets a fresh one.
///
/// Equality ignores `id`: two windows with the same contents and place are the same layout.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Floating<P> {
    /// Names the window across panel moves, and its OS window.
    /// Unique within a layout: [`Self::new`] draws a fresh id, [`Self::with_id`] sets one,
    /// loading keeps the saved id (or draws one when it is missing or taken), and merging
    /// windows keeps the receiving window's.
    pub id: u64,
    /// The window's groups, top to bottom, and its collapsed state. Its `width` is unused: the window's width is
    /// `rect`'s.
    pub column: Column<P>,
    /// The window's expanded area in logical points, in the application's viewport. Saved as
    /// `[x, y, width, height]`.
    pub rect: Rect,
    /// Where the window is on the desktop while it is in its own OS window: the record app windows keep too, placed
    /// with [`place`](crate::placement::place).
    pub native: Option<Placement>,
    /// The edge of the float area, in reading order, the window keeps to, as the area decided it from where the strip
    /// of the collapsed column was dropped ([`Action::MoveFloating`](super::Action::MoveFloating),
    /// [`Target::Float`](super::Target::Float)): the collapsed column shows against that edge of `rect`, and the
    /// title row's double arrow sits at that end. `None`: the edge nearer `rect`, which the area takes. Collapsing
    /// the column clears it, so the strip hangs from the edge the expanded window showed.
    pub edge: Option<DockEdge>,
}

impl<P: PartialEq> PartialEq for Floating<P> {
    fn eq(&self, other: &Self) -> bool {
        self.column == other.column && self.rect == other.rect && self.native == other.native && self.edge == other.edge
    }
}

impl<P> Node<P> {
    /// A group of tabs with tab `active` selected.
    pub fn tabs(panels: Vec<P>, active: usize) -> Self {
        Self::Tabs { panels, active }
    }

    /// Two children along `axis`, sharing the room by `size`.
    pub fn split(axis: Axis, size: SplitSize, first: Node<P>, second: Node<P>) -> Self {
        Self::Split { axis, size, first: Box::new(first), second: Box::new(second) }
    }
}

impl<P> StackEntry<P> {
    /// An open accordion section for `panel` with no preferred height.
    pub fn new(panel: P) -> Self {
        Self { panel, open: true, height: None }
    }

    /// The section as a group of its own: minimized when the section was closed, at the section's height.
    pub(super) fn into_group(self) -> Group<P> {
        Group { minimized: !self.open, height: self.height, ..Group::new(vec![self.panel], 0) }
    }
}

/// A window id no other window of this process has drawn, from a random start so ids saved
/// by earlier runs are unlikely to repeat.
pub(super) fn fresh_window_id() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    };
    static NEXT: OnceLock<AtomicU64> = OnceLock::new();
    let next = NEXT.get_or_init(|| AtomicU64::new(std::collections::hash_map::RandomState::new().build_hasher().finish() >> 1));
    next.fetch_add(1, Ordering::Relaxed)
}

impl<P> Floating<P> {
    /// A window holding `column`. Each call makes a new window, with a fresh [`Self::id`]: build
    /// a layout once and keep it, or give its windows ids with [`Self::with_id`]. A layout
    /// built anew each frame without them opens new OS windows each frame.
    pub fn new(column: Column<P>, rect: Rect) -> Self {
        Self { id: fresh_window_id(), column, rect, native: None, edge: None }
    }

    /// A window with one group. Each call makes a new window, as [`Self::new`] does.
    pub fn single(panels: Vec<P>, active: usize, rect: Rect) -> Self {
        Self::new(Column::new(vec![Group::new(panels, active)]), rect)
    }

    /// The window with `id`, for a layout the app builds in code, such as a default layout:
    /// built with the same id, it is the same window, and the same OS window, on every frame.
    /// Ids must be unique within a layout.
    pub fn with_id(mut self, id: u64) -> Self {
        self.id = id;
        self
    }

    /// Every panel in the window, top group first.
    pub fn panels(&self) -> impl Iterator<Item = &P> {
        self.column.panels()
    }

    /// A window of `groups`, stacked top to bottom, expanded. `None` without groups. Each call makes a new window,
    /// as [`Self::new`] does.
    pub fn stack(groups: Vec<Group<P>>, rect: Rect) -> Option<Self> {
        (!groups.is_empty()).then(|| Self::new(Column::new(groups), rect))
    }

    /// The window's groups, top to bottom.
    pub fn groups(&self) -> &[Group<P>] {
        &self.column.groups
    }

    /// True when a group of this window holds `panel`. Takes any borrowed form of `P`, such
    /// as `&str` for `String` panels.
    pub fn contains<Q: ?Sized + PartialEq>(&self, panel: &Q) -> bool
    where
        P: std::borrow::Borrow<Q>,
    {
        self.panels().any(|p| p.borrow() == panel)
    }
}

/// The saved shapes of a floating window, read by [`unique_ids`](floating_serde::unique_ids) into the windows of a
/// layout:
///
/// | Saved shape | Becomes |
/// |---|---|
/// | v1 `{panels, active, rect}` | one window, one group |
/// | v2 `{groups, rect}` | one window of those groups |
/// | v3 `{root, rect, native, minimized}` | a window per part the tree's top splits side by side, each at its share of `rect`; each part's leaves in reading order become its groups (an accordion entry a group, minimized when its section was closed); `minimized` minimizes every group |
/// | v4 `{column, rect, native, edge}` | one window |
///
/// The first window of a saved one keeps its id; the others get fresh ones.
pub(super) mod floating_serde {
    use emath::{Rect, pos2, vec2};
    use serde::de::IgnoredAny;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::{Column, DockEdge, Floating, Group, Node, fresh_window_id};
    use crate::dock::limits::MAX_NODES;
    use crate::dock::rect_serde::{from_array, to_array};
    use crate::dock::split::SplitSize;
    use crate::flow::Axis;
    use crate::placement::Placement;

    #[derive(Serialize)]
    struct V1<'a, P> {
        id: u64,
        panels: &'a [P],
        active: usize,
        rect: [f32; 4],
    }

    #[derive(Serialize)]
    struct V4<'a, P> {
        id: u64,
        column: &'a Column<P>,
        rect: [f32; 4],
        #[serde(skip_serializing_if = "Option::is_none")]
        native: Option<&'a Placement>,
        #[serde(skip_serializing_if = "Option::is_none")]
        edge: Option<DockEdge>,
    }

    /// One expanded column holding one group with no state of its own: what v1 can hold.
    fn plain<P>(column: &Column<P>, group: &Group<P>) -> bool {
        column.width.is_none()
            && !column.iconic
            && column.iconic_width.is_none()
            && column.steps.is_none()
            && !group.minimized
            && group.height.is_none()
            && group.flyout_width.is_none()
            && group.flyout_offset.is_none()
    }

    impl<P: Serialize> Serialize for Floating<P> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            match self.column.groups.as_slice() {
                [group] if self.native.is_none() && self.edge.is_none() && plain(&self.column, group) => {
                    V1 { id: self.id, panels: &group.panels, active: group.active, rect: to_array(self.rect) }.serialize(serializer)
                }
                _ => V4 { id: self.id, column: &self.column, rect: to_array(self.rect), native: self.native.as_ref(), edge: self.edge }
                    .serialize(serializer),
            }
        }
    }

    /// The fields of a saved window, whichever shape it was written in. A key is read as [`Serialize`] writes it:
    /// `id`, `column`, `root`, `groups`, `panels` and `rect` plain, and the optional `native` as an option, which
    /// RON marks with `Some` and JSON writes bare. A `native` that does not parse is no placement, and keys it does
    /// not know are skipped.
    struct Repr<P> {
        id: Option<u64>,
        column: Option<Column<P>>,
        root: Option<Node<P>>,
        groups: Option<Vec<Group<P>>>,
        panels: Option<Vec<P>>,
        active: usize,
        rect: Option<[f32; 4]>,
        native: Option<Placement>,
        minimized: bool,
        edge: Option<DockEdge>,
    }

    /// A saved value, or nothing in its place when it does not parse.
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Parsed<T> {
        Value(T),
        Unparsed(IgnoredAny),
    }

    impl<T> Parsed<T> {
        fn value(self) -> Option<T> {
            match self {
                Parsed::Value(value) => Some(value),
                Parsed::Unparsed(_) => None,
            }
        }
    }

    impl<'de, P: Deserialize<'de>> Deserialize<'de> for Repr<P> {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            use serde::de::{MapAccess, Visitor};
            use std::marker::PhantomData;

            const FIELDS: &[&str] = &["id", "column", "root", "groups", "panels", "active", "rect", "native", "minimized", "edge"];
            struct V<P>(PhantomData<P>);

            impl<'de, P: Deserialize<'de>> Visitor<'de> for V<P> {
                type Value = Repr<P>;

                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("a floating window")
                }

                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Repr<P>, A::Error> {
                    let mut repr = Repr {
                        id: None,
                        column: None,
                        root: None,
                        groups: None,
                        panels: None,
                        active: 0,
                        rect: None,
                        native: None,
                        minimized: false,
                        edge: None,
                    };
                    while let Some(key) = map.next_key::<String>()? {
                        match key.as_str() {
                            "id" => repr.id = Some(map.next_value()?),
                            "column" => repr.column = Some(map.next_value()?),
                            "root" => repr.root = Some(map.next_value()?),
                            "groups" => repr.groups = Some(map.next_value()?),
                            "panels" => repr.panels = Some(map.next_value()?),
                            "active" => repr.active = map.next_value()?,
                            "rect" => repr.rect = Some(map.next_value()?),
                            "native" => repr.native = map.next_value::<Option<Parsed<Placement>>>()?.and_then(Parsed::value),
                            "minimized" => repr.minimized = map.next_value()?,
                            "edge" => repr.edge = map.next_value()?,
                            _ => {
                                map.next_value::<IgnoredAny>()?;
                            }
                        }
                    }
                    Ok(repr)
                }
            }

            deserializer.deserialize_struct("Floating", FIELDS, V(PhantomData))
        }
    }

    impl<P> Repr<P> {
        /// The windows the saved window becomes, first to last.
        fn into_windows<E: serde::de::Error>(self) -> Result<Vec<Floating<P>>, E> {
            let Repr { id, column, root, groups, panels, active, rect, native, minimized, edge } = self;
            let rect = from_array(rect.ok_or_else(|| E::missing_field("rect"))?);
            let columns = match (column, root, groups, panels) {
                (Some(column), None, None, None) => vec![(column, rect)],
                (None, Some(root), None, None) => {
                    let mut parts = Vec::new();
                    side_by_side(root, rect, &mut parts);
                    parts
                        .into_iter()
                        .map(|(node, rect)| {
                            let mut groups = node_groups(node);
                            groups.iter_mut().for_each(|group| group.minimized |= minimized);
                            (Column::new(groups), rect)
                        })
                        .collect()
                }
                (None, None, Some(groups), None) => vec![(Column::new(groups), rect)],
                (None, None, None, Some(panels)) => vec![(Column::new(vec![Group::new(panels, active)]), rect)],
                (None, None, None, None) => return Err(E::missing_field("column")),
                _ => return Err(E::duplicate_field("column")),
            };
            if columns.iter().any(|(column, _)| column.groups.is_empty()) {
                return Err(E::invalid_length(0, &"at least one group"));
            }
            let native = native.and_then(|native| native.repaired());
            let mut id = Some(id.unwrap_or_else(fresh_window_id));
            Ok(columns
                .into_iter()
                .map(|(column, rect)| Floating { id: id.take().unwrap_or_else(fresh_window_id), column, rect, native: native.clone(), edge })
                .collect())
        }
    }

    /// The parts a v3 window tree's top splits side by side, in reading order, each with its share of `rect` by
    /// the split sizes. Every part keeps some width, so the window it becomes is valid.
    fn side_by_side<P>(node: Node<P>, rect: Rect, parts: &mut Vec<(Node<P>, Rect)>) {
        match node {
            Node::Split { axis: Axis::X, size, first, second } => {
                let width = rect.width();
                let lead = match size {
                    SplitSize::Ratio(ratio) => width * ratio,
                    SplitSize::FixedFirst(first) => first,
                    SplitSize::FixedSecond(second) => width - second,
                };
                let lead = if lead.is_finite() { lead } else { width / 2.0 };
                let lead = lead.min(width - 1.0).max(1.0);
                side_by_side(*first, Rect::from_min_size(rect.min, vec2(lead, rect.height())), parts);
                side_by_side(*second, Rect::from_min_max(pos2(rect.min.x + lead, rect.min.y), rect.max), parts);
            }
            node => parts.push((node, rect)),
        }
    }

    /// The groups of a v3 window part in reading order: a `Tabs` leaf is a group, each accordion entry a group of
    /// its own. Stops at [`MAX_NODES`] for an invalid tree.
    fn node_groups<P>(node: Node<P>) -> Vec<Group<P>> {
        let mut groups = Vec::new();
        let mut visit = vec![node];
        let mut visited = 0usize;
        while let Some(node) = visit.pop() {
            visited = visited.saturating_add(1);
            if visited > MAX_NODES {
                break;
            }
            match node {
                Node::Split { first, second, .. } => {
                    visit.push(*second);
                    visit.push(*first);
                }
                Node::Tabs { panels, active } => groups.push(Group::new(panels, active)),
                Node::Stack { entries } => groups.extend(entries.into_iter().map(super::StackEntry::into_group)),
                Node::Dock { columns, .. } => groups.extend(columns.into_iter().flat_map(|column| column.groups)),
            }
        }
        groups
    }

    /// Floating windows as saved, each read into the windows it becomes, with a fresh id for each one whose id an
    /// earlier window took.
    pub(in crate::dock) fn unique_ids<'de, D: Deserializer<'de>, P: Deserialize<'de>>(deserializer: D) -> Result<Vec<Floating<P>>, D::Error> {
        let mut windows = Vec::new();
        for saved in Vec::<Repr<P>>::deserialize(deserializer)? {
            windows.extend(saved.into_windows::<D::Error>()?);
        }
        let mut seen = std::collections::HashSet::new();
        for window in &mut windows {
            while !seen.insert(window.id) {
                window.id = fresh_window_id();
            }
        }
        Ok(windows)
    }
}

impl<P> Node<P> {
    /// Builds [`Node::Stack`].
    pub fn stack(entries: Vec<StackEntry<P>>) -> Self {
        Self::Stack { entries }
    }

    /// Builds [`Node::Dock`].
    pub fn dock(side: DockEdge, columns: Vec<Column<P>>) -> Self {
        Self::Dock { side, columns }
    }
}
