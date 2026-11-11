//! The docked tree, its leaves and the floating windows.

use emath::Rect;

use crate::{flow::Axis, placement::Placement};

use super::{
    edit::{stack_tree, tree_groups, tree_panels},
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

/// A floating window: a tree of groups like the docked layout's, without docks.
///
/// Serde reads the v1 shape `{panels, active, rect}` as one group, and a stack of groups
/// `{groups, rect}` as a vertical split of them, sized by their heights. A window with one
/// group (not minimized, no native rect) writes the v1 shape, so older builds can read files
/// that use no newer feature. Every window writes its `id`; a window read without one gets a
/// fresh one.
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
    /// `Split`, `Tabs` and `Stack` nodes; a `Dock` is invalid here.
    pub root: Node<P>,
    /// The window's area in logical points, in the application's viewport. Saved as `[x, y, width, height]`.
    pub rect: Rect,
    /// Where the window is on the desktop while it is in its own OS window: the record app windows keep too, placed
    /// with [`place`](crate::placement::place).
    pub native: Option<Placement>,
    /// Only the title bar shows.
    pub minimized: bool,
}

impl<P: PartialEq> PartialEq for Floating<P> {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root && self.rect == other.rect && self.native == other.native && self.minimized == other.minimized
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
    /// A window holding `root`. Each call makes a new window, with a fresh [`Self::id`]: build
    /// a layout once and keep it, or give its windows ids with [`Self::with_id`]. A layout
    /// built anew each frame without them opens new OS windows each frame.
    pub fn new(root: Node<P>, rect: Rect) -> Self {
        Self { id: fresh_window_id(), root, rect, native: None, minimized: false }
    }

    /// A window with one group. Each call makes a new window, as [`Self::new`] does.
    pub fn single(panels: Vec<P>, active: usize, rect: Rect) -> Self {
        Self::new(Node::Tabs { panels, active }, rect)
    }

    /// The window with `id`, for a layout the app builds in code, such as a default layout:
    /// built with the same id, it is the same window, and the same OS window, on every frame.
    /// Ids must be unique within a layout.
    pub fn with_id(mut self, id: u64) -> Self {
        self.id = id;
        self
    }

    /// Every panel in the window, in tree order.
    pub fn panels(&self) -> impl Iterator<Item = &P> {
        tree_panels(&self.root).into_iter()
    }

    /// Groups stacked top to bottom, as a v1 layout stored them: each group over a vertical
    /// split of the ones below it, sized by their heights. `None` without groups. Each call
    /// makes a new window, as [`Self::new`] does.
    pub fn stack(groups: Vec<Group<P>>, rect: Rect) -> Option<Self> {
        stack_tree(groups).map(|root| Self::new(root, rect))
    }

    /// The window's groups, first to last.
    pub fn groups(&self) -> Vec<Group<P>>
    where
        P: Clone,
    {
        tree_groups(&self.root)
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

pub(super) mod floating_serde {
    use serde::de::IgnoredAny;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::{Floating, Group, Node, fresh_window_id};
    use crate::dock::edit::stack_tree;
    use crate::dock::rect_serde::{from_array, to_array};
    use crate::placement::Placement;

    #[derive(Serialize)]
    struct V1<'a, P> {
        id: u64,
        panels: &'a [P],
        active: usize,
        rect: [f32; 4],
    }

    #[derive(Serialize)]
    struct Tree<'a, P> {
        id: u64,
        root: &'a Node<P>,
        rect: [f32; 4],
        #[serde(skip_serializing_if = "Option::is_none")]
        native: Option<&'a Placement>,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        minimized: bool,
    }

    impl<P: Serialize> Serialize for Floating<P> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            match &self.root {
                Node::Tabs { panels, active } if !self.minimized && self.native.is_none() => {
                    V1 { id: self.id, panels, active: *active, rect: to_array(self.rect) }.serialize(serializer)
                }
                root => Tree { id: self.id, root, rect: to_array(self.rect), native: self.native.as_ref(), minimized: self.minimized }
                    .serialize(serializer),
            }
        }
    }

    /// The fields of a saved window, whichever of the three shapes it was written in. A key is
    /// read as [`Serialize`] writes it: `id`, `root`, `groups`, `panels` and `rect` plain, and the
    /// optional `native` as an option, which RON marks with `Some` and JSON writes bare. A
    /// `native` that does not parse is no placement, and keys it does not know are skipped.
    struct Repr<P> {
        id: Option<u64>,
        root: Option<Node<P>>,
        groups: Option<Vec<Group<P>>>,
        panels: Option<Vec<P>>,
        active: usize,
        rect: Option<[f32; 4]>,
        native: Option<Placement>,
        minimized: bool,
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

            const FIELDS: &[&str] = &["id", "root", "groups", "panels", "active", "rect", "native", "minimized"];
            struct V<P>(PhantomData<P>);

            impl<'de, P: Deserialize<'de>> Visitor<'de> for V<P> {
                type Value = Repr<P>;

                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("a floating window")
                }

                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Repr<P>, A::Error> {
                    let mut repr = Repr { id: None, root: None, groups: None, panels: None, active: 0, rect: None, native: None, minimized: false };
                    while let Some(key) = map.next_key::<String>()? {
                        match key.as_str() {
                            "id" => repr.id = Some(map.next_value()?),
                            "root" => repr.root = Some(map.next_value()?),
                            "groups" => repr.groups = Some(map.next_value()?),
                            "panels" => repr.panels = Some(map.next_value()?),
                            "active" => repr.active = map.next_value()?,
                            "rect" => repr.rect = Some(map.next_value()?),
                            "native" => repr.native = map.next_value::<Option<Parsed<Placement>>>()?.and_then(Parsed::value),
                            "minimized" => repr.minimized = map.next_value()?,
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

    /// Floating windows with a fresh id for each one whose id an earlier window took.
    pub(in crate::dock) fn unique_ids<'de, D: Deserializer<'de>, P: Deserialize<'de>>(deserializer: D) -> Result<Vec<Floating<P>>, D::Error> {
        let mut windows = Vec::<Floating<P>>::deserialize(deserializer)?;
        let mut seen = std::collections::HashSet::new();
        for window in &mut windows {
            while !seen.insert(window.id) {
                window.id = fresh_window_id();
            }
        }
        Ok(windows)
    }

    impl<'de, P: Deserialize<'de>> Deserialize<'de> for Floating<P> {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            let repr = Repr::<P>::deserialize(deserializer)?;
            let mut minimized = repr.minimized;
            let root = match (repr.root, repr.groups, repr.panels) {
                (Some(root), None, None) => root,
                (None, Some(groups), None) => {
                    minimized |= !groups.is_empty() && groups.iter().all(|g| g.minimized);
                    stack_tree(groups).ok_or_else(|| serde::de::Error::invalid_length(0, &"at least one group"))?
                }
                (None, None, Some(panels)) => Node::Tabs { panels, active: repr.active },
                (None, None, None) => return Err(serde::de::Error::missing_field("root")),
                _ => return Err(serde::de::Error::duplicate_field("root")),
            };
            Ok(Floating {
                id: repr.id.unwrap_or_else(fresh_window_id),
                root,
                rect: from_array(repr.rect.ok_or_else(|| serde::de::Error::missing_field("rect"))?),
                native: repr.native.and_then(|native| native.repaired()),
                minimized,
            })
        }
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
