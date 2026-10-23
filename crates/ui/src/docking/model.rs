use std::{collections::HashSet, fmt, hash::Hash};

use crate::layout::{SplitAxis, SplitSize};

/// Limits are checked before cloning or recursively modifying an application layout.
pub const MAX_DEPTH: usize = 64;
pub const MAX_PANELS: usize = 4096;
const MAX_NODES: usize = MAX_PANELS * 2;
const MAX_EXTENT: f32 = 1_000_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Zone {
    Center,
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Node<P> {
    Split {
        axis: SplitAxis,
        size: SplitSize,
        first: Box<Node<P>>,
        second: Box<Node<P>>,
    },
    Tabs {
        panels: Vec<P>,
        active: usize,
    },
    /// An accordion keeps its open state and preferred heights when other panels move.
    Stack {
        entries: Vec<StackEntry<P>>,
    },
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StackEntry<P> {
    pub panel: P,
    pub open: bool,
    pub height: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Floating<P> {
    pub panels: Vec<P>,
    pub active: usize,
    /// `[x, y, width, height]` in logical points, in the application's egui viewport.
    pub rect: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Layout<P> {
    pub root: Option<Node<P>>,
    pub floating: Vec<Floating<P>>,
}

impl<P> Default for Layout<P> {
    fn default() -> Self {
        Self { root: None, floating: Vec::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Placement<P> {
    /// Insert before this existing destination member, or append. Also reorders a group.
    Tab { before: Option<P> },
    /// Center inserts after the anchor; sides create a new half-size split.
    Split(Zone),
}

/// A closed panel's former placement. Applications may retain this beside visibility flags.
/// Anchors are stable panel identities rather than mutable tree indices.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(bound(deserialize = "P: serde::Deserialize<'de>")))]
pub struct Location<P> {
    pub floating: Option<[f32; 4]>,
    pub anchor: Option<P>,
    pub placement: Placement<P>,
    pub size: Option<SplitSize>,
    /// Accordion state to restore if the destination is still an accordion.
    pub stack: Option<(bool, Option<f32>)>,
    /// A split's complete sibling subtree. This retains placement around a nested workspace,
    /// rather than accidentally inserting beside only its first leaf when restoring.
    #[cfg_attr(feature = "serde", serde(default))]
    pub sibling: Vec<P>,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Action<P> {
    Move {
        panel: P,
        anchor: P,
        placement: Placement<P>,
    },
    Float {
        panel: P,
        rect: [f32; 4],
    },
    Close {
        panel: P,
    },
    Activate {
        panel: P,
    },
    /// Reveal a registered, currently absent panel. Applications validate their panel registry.
    Open {
        panel: P,
        anchor: Option<P>,
    },
    /// Reveal an absent panel beside a protected canvas or at another explicit destination.
    OpenAt {
        panel: P,
        anchor: P,
        placement: Placement<P>,
    },
    /// Reposition/resize a floating group, identified by one of its panels.
    MoveFloating {
        panel: P,
        rect: [f32; 4],
    },
    /// Path from the root: false is the first child, true the second.
    ResizeSplit {
        path: Vec<bool>,
        size: SplitSize,
    },
    SetStackOpen {
        panel: P,
        open: bool,
    },
    ResizeStack {
        panel: P,
        height: Option<f32>,
    },
}

/// Rendering and command adapters can protect a document canvas while leaving panels movable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Permissions {
    pub movable: bool,
    pub floatable: bool,
    pub closable: bool,
    pub accepts_tabs: bool,
}

impl Default for Permissions {
    fn default() -> Self {
        Self { movable: true, floatable: true, closable: true, accepts_tabs: true }
    }
}

impl Permissions {
    pub const PROTECTED: Self = Self { movable: false, floatable: false, closable: false, accepts_tabs: false };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockError {
    LimitExceeded,
    DuplicatePanel,
    MissingPanel,
    MissingTarget,
    InvalidSelection,
    InvalidGeometry,
    InvalidPlacement,
    NotAllowed,
}

impl fmt::Display for DockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::LimitExceeded => "docking layout exceeds its size or depth limit",
            Self::DuplicatePanel => "panel occurs in more than one docking location",
            Self::MissingPanel => "source panel is not present in the docking layout",
            Self::MissingTarget => "docking destination is no longer present",
            Self::InvalidSelection => "active docking tab is out of bounds",
            Self::InvalidGeometry => "docking geometry is not finite and bounded",
            Self::InvalidPlacement => "docking destination does not support this placement",
            Self::NotAllowed => "this panel does not allow the requested docking operation",
        })
    }
}

impl std::error::Error for DockError {}

pub(super) fn valid_rect(rect: [f32; 4]) -> bool {
    rect.iter().all(|v| v.is_finite() && v.abs() <= MAX_EXTENT) && rect.get(2).is_some_and(|v| *v > 0.0) && rect.get(3).is_some_and(|v| *v > 0.0)
}

fn valid_size(size: SplitSize) -> bool {
    match size {
        SplitSize::Ratio(r) => r.is_finite() && (0.0..=1.0).contains(&r),
        SplitSize::FixedFirst(v) | SplitSize::FixedSecond(v) => v.is_finite() && (0.0..=MAX_EXTENT).contains(&v),
    }
}

fn valid_height(height: Option<f32>) -> bool {
    height.is_none_or(|v| v.is_finite() && (0.0..=MAX_EXTENT).contains(&v))
}

fn valid_active(len: usize, active: usize) -> bool {
    active < len || (len == 0 && active == 0)
}

impl<P: Clone + Eq + Hash> Layout<P> {
    /// Check depth, allocation bounds, geometry, selection and globally unique panel ownership.
    /// Traversal is iterative, including for an invalid, excessively deep supplied tree.
    pub fn validate(&self) -> Result<(), DockError> {
        if self.floating.len() > MAX_PANELS {
            return Err(DockError::LimitExceeded);
        }
        let mut seen = HashSet::new();
        let mut count = 0usize;
        let mut visit = self.root.as_ref().map(|n| vec![(n, 1usize)]).unwrap_or_default();
        while let Some((node, depth)) = visit.pop() {
            count = count.saturating_add(1);
            if depth > MAX_DEPTH || count > MAX_NODES {
                return Err(DockError::LimitExceeded);
            }
            match node {
                Node::Split { size, first, second, .. } => {
                    if !valid_size(*size) {
                        return Err(DockError::InvalidGeometry);
                    }
                    visit.push((second, depth + 1));
                    visit.push((first, depth + 1));
                }
                Node::Tabs { panels, active } => {
                    check_tabs(panels, *active, &mut seen)?;
                }
                Node::Stack { entries } => {
                    if entries.len() > MAX_PANELS {
                        return Err(DockError::LimitExceeded);
                    }
                    for entry in entries {
                        if !valid_height(entry.height) {
                            return Err(DockError::InvalidGeometry);
                        }
                        check_panel(&entry.panel, &mut seen)?;
                    }
                }
            }
        }
        for floating in &self.floating {
            if !valid_rect(floating.rect) || floating.panels.is_empty() {
                return Err(DockError::InvalidGeometry);
            }
            check_tabs(&floating.panels, floating.active, &mut seen)?;
        }
        Ok(())
    }

    /// Iterate identities without recursion. On excessive input returns no more than the limit.
    pub fn panels(&self) -> Vec<&P> {
        let mut panels = Vec::new();
        let mut visit: Vec<&Node<P>> = self.root.iter().collect();
        let mut visited = 0usize;
        while let Some(node) = visit.pop() {
            visited = visited.saturating_add(1);
            if visited > MAX_NODES || panels.len() >= MAX_PANELS {
                break;
            }
            match node {
                Node::Split { first, second, .. } => {
                    visit.push(second);
                    visit.push(first);
                }
                Node::Tabs { panels: group, .. } => panels.extend(group.iter().take(MAX_PANELS - panels.len())),
                Node::Stack { entries } => panels.extend(entries.iter().map(|e| &e.panel).take(MAX_PANELS - panels.len())),
            }
        }
        for group in self.floating.iter().take(MAX_PANELS) {
            panels.extend(group.panels.iter().take(MAX_PANELS - panels.len()));
        }
        panels
    }

    pub fn contains(&self, panel: &P) -> bool {
        self.panels().contains(&panel)
    }

    /// Capture a panel's nearest surviving group/split neighbor before hiding it.
    pub fn location(&self, panel: &P) -> Result<Location<P>, DockError> {
        self.validate()?;
        if let Some(group) = self.floating.iter().find(|g| g.panels.contains(panel)) {
            let mut location = group_location(&group.panels, panel).ok_or(DockError::MissingPanel)?;
            location.floating = Some(group.rect);
            return Ok(location);
        }
        self.root.as_ref().and_then(|node| node_location(node, panel)).ok_or(DockError::MissingPanel)
    }

    /// Restore an absent panel at a captured location. Missing anchors are reported without
    /// changes, allowing the app to choose an explicit default. Unrelated tabs retain selection.
    pub fn restore(&mut self, panel: P, location: &Location<P>) -> Result<(), DockError> {
        self.validate()?;
        if self.contains(&panel) {
            return Err(DockError::DuplicatePanel);
        }
        if location.size.is_some_and(|s| !valid_size(s)) || location.stack.is_some_and(|(_, h)| !valid_height(h)) {
            return Err(DockError::InvalidGeometry);
        }
        let mut proposed = self.clone();
        if let Some(rect) = location.floating {
            if !valid_rect(rect) {
                return Err(DockError::InvalidGeometry);
            }
            if let Some(anchor) = &location.anchor {
                if !proposed.floating.iter().any(|g| g.panels.contains(anchor)) {
                    return Err(DockError::MissingTarget);
                }
                proposed.apply_inner(Action::OpenAt { panel: panel.clone(), anchor: anchor.clone(), placement: location.placement.clone() })?;
            } else {
                proposed.floating.push(Floating { panels: vec![panel.clone()], active: 0, rect });
            }
        } else if let Some(anchor) = &location.anchor {
            if !proposed.contains(anchor) {
                return Err(DockError::MissingTarget);
            }
            if let Placement::Split(zone @ (Zone::Left | Zone::Right | Zone::Top | Zone::Bottom)) = location.placement
                && !location.sibling.is_empty()
            {
                if location.sibling.len() > MAX_PANELS {
                    return Err(DockError::LimitExceeded);
                }
                let members: HashSet<&P> = location.sibling.iter().collect();
                if members.len() != location.sibling.len() || !members.contains(anchor) {
                    return Err(DockError::InvalidPlacement);
                }
                let inserted = proposed
                    .root
                    .as_mut()
                    .is_some_and(|root| restore_around_subtree(root, &panel, &members, zone, location.size.unwrap_or(SplitSize::Ratio(0.5))));
                if !inserted {
                    return Err(DockError::MissingTarget);
                }
            } else {
                proposed.apply_inner(Action::OpenAt { panel: panel.clone(), anchor: anchor.clone(), placement: location.placement.clone() })?;
                if let Some(size) = location.size
                    && let Some(root) = proposed.root.as_mut()
                {
                    restore_parent_size(root, &panel, size);
                }
            }
        } else if proposed.root.is_none() {
            proposed.root = Some(Node::Tabs { panels: vec![panel.clone()], active: 0 });
        } else {
            return Err(DockError::MissingTarget);
        }
        if let Some((open, height)) = location.stack
            && let Some(root) = proposed.root.as_mut()
        {
            restore_stack_kind(root, &panel);
            if let Some(entry) = stack_entry_mut(root, &panel) {
                entry.open = open;
                entry.height = height;
            }
        }
        proposed.validate()?;
        *self = proposed;
        Ok(())
    }

    pub fn apply(&mut self, action: Action<P>) -> Result<(), DockError> {
        self.apply_with_permissions(action, |_| Permissions::default())
    }

    /// Check application policy before making an atomic change. The callback must be stable
    /// throughout one call. Registry validation remains with the application, particularly Open.
    pub fn apply_with_permissions(&mut self, action: Action<P>, permissions: impl Fn(&P) -> Permissions) -> Result<(), DockError> {
        self.validate()?;
        match &action {
            Action::Move { panel, anchor, placement } | Action::OpenAt { panel, anchor, placement } => {
                if !permissions(panel).movable {
                    return Err(DockError::NotAllowed);
                }
                let tab = matches!(placement, Placement::Tab { .. } | Placement::Split(Zone::Center));
                if tab && !permissions(anchor).accepts_tabs {
                    return Err(DockError::NotAllowed);
                }
            }
            Action::Float { panel, .. } if !permissions(panel).floatable || !permissions(panel).movable => return Err(DockError::NotAllowed),
            Action::Close { panel } if !permissions(panel).closable => return Err(DockError::NotAllowed),
            // The rectangle belongs to the whole floating group, so every member must allow it.
            Action::MoveFloating { panel, .. }
                if !permissions(panel).movable
                    || self.floating.iter().find(|g| g.panels.contains(panel)).is_some_and(|g| g.panels.iter().any(|p| !permissions(p).movable)) =>
            {
                return Err(DockError::NotAllowed);
            }
            Action::Open { anchor: Some(anchor), .. } if !permissions(anchor).accepts_tabs => return Err(DockError::NotAllowed),
            _ => {}
        }
        let mut proposed = self.clone();
        proposed.apply_inner(action)?;
        proposed.validate()?;
        *self = proposed;
        Ok(())
    }

    fn apply_inner(&mut self, action: Action<P>) -> Result<(), DockError> {
        match action {
            Action::Move { panel, anchor, placement } => {
                if panel == anchor {
                    return Err(DockError::InvalidPlacement);
                }
                if !self.contains(&anchor) {
                    return Err(DockError::MissingTarget);
                }
                if let Placement::Tab { before: Some(before) } = &placement {
                    if before == &panel {
                        // A tab dropped immediately before itself has not moved.
                        return if self.same_group(&panel, &anchor) { Ok(()) } else { Err(DockError::InvalidPlacement) };
                    }
                    if !self.same_group(before, &anchor) {
                        return Err(DockError::MissingTarget);
                    }
                }
                let entry = self.take(&panel).ok_or(DockError::MissingPanel)?;
                self.insert(entry, &anchor, placement)
            }
            Action::Float { panel, rect } => {
                if !valid_rect(rect) {
                    return Err(DockError::InvalidGeometry);
                }
                let entry = self.take(&panel).ok_or(DockError::MissingPanel)?;
                self.floating.push(Floating { panels: vec![entry.panel], active: 0, rect });
                Ok(())
            }
            Action::Close { panel } => self.take(&panel).map(|_| ()).ok_or(DockError::MissingPanel),
            Action::Activate { panel } => {
                if self.root.as_mut().is_some_and(|node| activate(node, &panel)) {
                    return Ok(());
                }
                for group in &mut self.floating {
                    if let Some(i) = group.panels.iter().position(|p| p == &panel) {
                        group.active = i;
                        return Ok(());
                    }
                }
                Err(DockError::MissingPanel)
            }
            Action::Open { panel, anchor } => {
                if self.contains(&panel) {
                    return self.apply_inner(Action::Activate { panel });
                }
                let entry = StackEntry { panel, open: true, height: None };
                if let Some(anchor) = anchor {
                    self.insert(entry, &anchor, Placement::Tab { before: None })
                } else if self.root.is_none() {
                    self.root = Some(Node::Tabs { panels: vec![entry.panel], active: 0 });
                    Ok(())
                } else {
                    Err(DockError::MissingTarget)
                }
            }
            Action::OpenAt { panel, anchor, placement } => {
                if self.contains(&panel) {
                    return self.apply_inner(Action::Move { panel, anchor, placement });
                }
                if let Placement::Tab { before: Some(before) } = &placement
                    && !self.same_group(before, &anchor)
                {
                    return Err(DockError::MissingTarget);
                }
                self.insert(StackEntry { panel, open: true, height: None }, &anchor, placement)
            }
            Action::MoveFloating { panel, rect } => {
                if !valid_rect(rect) {
                    return Err(DockError::InvalidGeometry);
                }
                let group = self.floating.iter_mut().find(|g| g.panels.contains(&panel)).ok_or(DockError::MissingPanel)?;
                group.rect = rect;
                Ok(())
            }
            Action::ResizeSplit { path, size } => {
                if path.len() >= MAX_DEPTH {
                    return Err(DockError::LimitExceeded);
                }
                if !valid_size(size) {
                    return Err(DockError::InvalidGeometry);
                }
                let mut node = self.root.as_mut().ok_or(DockError::MissingTarget)?;
                for second_child in path {
                    let Node::Split { first, second, .. } = node else { return Err(DockError::MissingTarget) };
                    node = if second_child { second } else { first };
                }
                let Node::Split { size: old, .. } = node else { return Err(DockError::MissingTarget) };
                *old = size;
                Ok(())
            }
            Action::SetStackOpen { panel, open } => {
                let entry = self.root.as_mut().and_then(|n| stack_entry_mut(n, &panel)).ok_or(DockError::MissingPanel)?;
                entry.open = open;
                Ok(())
            }
            Action::ResizeStack { panel, height } => {
                if !valid_height(height) {
                    return Err(DockError::InvalidGeometry);
                }
                let entry = self.root.as_mut().and_then(|n| stack_entry_mut(n, &panel)).ok_or(DockError::MissingPanel)?;
                entry.height = height;
                Ok(())
            }
        }
    }

    fn same_group(&self, panel: &P, anchor: &P) -> bool {
        fn in_node<P: Eq>(node: &Node<P>, panel: &P, anchor: &P) -> bool {
            match node {
                Node::Tabs { panels, .. } => panels.contains(panel) && panels.contains(anchor),
                Node::Stack { entries } => entries.iter().any(|e| &e.panel == panel) && entries.iter().any(|e| &e.panel == anchor),
                Node::Split { first, second, .. } => in_node(first, panel, anchor) || in_node(second, panel, anchor),
            }
        }
        self.root.as_ref().is_some_and(|n| in_node(n, panel, anchor))
            || self.floating.iter().any(|g| g.panels.contains(panel) && g.panels.contains(anchor))
    }

    fn take(&mut self, panel: &P) -> Option<StackEntry<P>> {
        let mut removed = None;
        self.root = self.root.take().and_then(|n| remove_node(n, panel, &mut removed));
        if removed.is_some() {
            return removed;
        }
        for group in &mut self.floating {
            if let Some(p) = remove_tab(&mut group.panels, &mut group.active, panel) {
                removed = Some(StackEntry { panel: p, open: true, height: None });
                break;
            }
        }
        self.floating.retain(|g| !g.panels.is_empty());
        removed
    }

    fn insert(&mut self, entry: StackEntry<P>, anchor: &P, placement: Placement<P>) -> Result<(), DockError> {
        if let Some(group) = self.floating.iter_mut().find(|g| g.panels.contains(anchor)) {
            let index = match placement {
                Placement::Tab { before } => before.as_ref().and_then(|p| group.panels.iter().position(|q| q == p)).unwrap_or(group.panels.len()),
                Placement::Split(Zone::Center) => group.panels.iter().position(|p| p == anchor).map_or(group.panels.len(), |i| i + 1),
                Placement::Split(_) => return Err(DockError::InvalidPlacement),
            };
            group.panels.insert(index, entry.panel);
            group.active = index;
            return Ok(());
        }
        if let Some(root) = self.root.as_mut()
            && insert_node(root, &entry, anchor, &placement)
        {
            return Ok(());
        }
        Err(DockError::MissingTarget)
    }
}

fn check_panel<'a, P: Eq + Hash>(panel: &'a P, seen: &mut HashSet<&'a P>) -> Result<(), DockError> {
    if seen.len() >= MAX_PANELS {
        return Err(DockError::LimitExceeded);
    }
    if !seen.insert(panel) {
        return Err(DockError::DuplicatePanel);
    }
    Ok(())
}

fn check_tabs<'a, P: Eq + Hash>(panels: &'a [P], active: usize, seen: &mut HashSet<&'a P>) -> Result<(), DockError> {
    if panels.len() > MAX_PANELS {
        return Err(DockError::LimitExceeded);
    }
    if !valid_active(panels.len(), active) {
        return Err(DockError::InvalidSelection);
    }
    for panel in panels {
        check_panel(panel, seen)?;
    }
    Ok(())
}

fn remove_tab<P: Clone + Eq>(panels: &mut Vec<P>, active: &mut usize, panel: &P) -> Option<P> {
    let index = panels.iter().position(|p| p == panel)?;
    let selected = panels.get(*active).cloned();
    let removed = panels.remove(index);
    *active = selected.as_ref().and_then(|p| panels.iter().position(|q| q == p)).unwrap_or(index.min(panels.len().saturating_sub(1)));
    Some(removed)
}

fn remove_node<P: Clone + Eq>(node: Node<P>, panel: &P, removed: &mut Option<StackEntry<P>>) -> Option<Node<P>> {
    match node {
        Node::Tabs { mut panels, mut active } => {
            if let Some(panel) = remove_tab(&mut panels, &mut active, panel) {
                *removed = Some(StackEntry { panel, open: true, height: None });
            }
            (!panels.is_empty()).then_some(Node::Tabs { panels, active })
        }
        Node::Stack { mut entries } => {
            if let Some(i) = entries.iter().position(|e| &e.panel == panel) {
                *removed = Some(entries.remove(i));
            }
            (!entries.is_empty()).then_some(Node::Stack { entries })
        }
        Node::Split { axis, size, first, second } => {
            let first = remove_node(*first, panel, removed);
            let second = remove_node(*second, panel, removed);
            match (first, second) {
                (Some(first), Some(second)) => Some(Node::Split { axis, size, first: Box::new(first), second: Box::new(second) }),
                (node, None) | (None, node) => node,
            }
        }
    }
}

fn activate<P: Eq>(node: &mut Node<P>, panel: &P) -> bool {
    match node {
        Node::Tabs { panels, active } => {
            if let Some(i) = panels.iter().position(|p| p == panel) {
                *active = i;
                true
            } else {
                false
            }
        }
        Node::Stack { entries } => {
            if let Some(entry) = entries.iter_mut().find(|e| &e.panel == panel) {
                entry.open = true;
                true
            } else {
                false
            }
        }
        Node::Split { first, second, .. } => activate(first, panel) || activate(second, panel),
    }
}

fn stack_entry_mut<'a, P: Eq>(node: &'a mut Node<P>, panel: &P) -> Option<&'a mut StackEntry<P>> {
    match node {
        Node::Stack { entries } => entries.iter_mut().find(|e| &e.panel == panel),
        Node::Split { first, second, .. } => stack_entry_mut(first, panel).or_else(|| stack_entry_mut(second, panel)),
        Node::Tabs { .. } => None,
    }
}

fn insert_node<P: Clone + Eq>(node: &mut Node<P>, entry: &StackEntry<P>, anchor: &P, placement: &Placement<P>) -> bool {
    let here = match node {
        Node::Split { first, second, .. } => return insert_node(first, entry, anchor, placement) || insert_node(second, entry, anchor, placement),
        Node::Tabs { panels, .. } => panels.contains(anchor),
        Node::Stack { entries } => entries.iter().any(|e| &e.panel == anchor),
    };
    if !here {
        return false;
    }
    if let Placement::Split(zone @ (Zone::Left | Zone::Right | Zone::Top | Zone::Bottom)) = placement {
        let old = std::mem::replace(node, Node::Tabs { panels: Vec::new(), active: 0 });
        let new = Node::Tabs { panels: vec![entry.panel.clone()], active: 0 };
        let axis = if matches!(zone, Zone::Top | Zone::Bottom) { SplitAxis::Vertical } else { SplitAxis::Horizontal };
        let (first, second) = if matches!(zone, Zone::Top | Zone::Left) { (new, old) } else { (old, new) };
        *node = Node::Split { axis, size: SplitSize::Ratio(0.5), first: Box::new(first), second: Box::new(second) };
        return true;
    }
    match node {
        Node::Tabs { panels, active } => {
            let index = match placement {
                Placement::Tab { before } => before.as_ref().and_then(|p| panels.iter().position(|q| q == p)).unwrap_or(panels.len()),
                Placement::Split(_) => panels.iter().position(|p| p == anchor).map_or(panels.len(), |i| i + 1),
            };
            panels.insert(index, entry.panel.clone());
            *active = index;
        }
        Node::Stack { entries } => {
            let index = match placement {
                Placement::Tab { before } => before.as_ref().and_then(|p| entries.iter().position(|e| &e.panel == p)).unwrap_or(entries.len()),
                Placement::Split(_) => entries.iter().position(|e| &e.panel == anchor).map_or(entries.len(), |i| i + 1),
            };
            entries.insert(index, entry.clone());
        }
        Node::Split { .. } => return false,
    }
    true
}

fn group_location<P: Clone + Eq>(panels: &[P], panel: &P) -> Option<Location<P>> {
    let index = panels.iter().position(|p| p == panel)?;
    let anchor = panels.iter().find(|p| *p != panel).cloned();
    let before = panels.get(index + 1).cloned();
    Some(Location { floating: None, anchor, placement: Placement::Tab { before }, size: None, stack: None, sibling: Vec::new() })
}

fn first_panel<P>(node: &Node<P>) -> Option<&P> {
    match node {
        Node::Tabs { panels, .. } => panels.first(),
        Node::Stack { entries } => entries.first().map(|e| &e.panel),
        Node::Split { first, second, .. } => first_panel(first).or_else(|| first_panel(second)),
    }
}

fn node_location<P: Clone + Eq>(node: &Node<P>, panel: &P) -> Option<Location<P>> {
    match node {
        Node::Tabs { panels, .. } => group_location(panels, panel),
        Node::Stack { entries } => {
            let entry = entries.iter().find(|e| &e.panel == panel)?;
            let panels: Vec<P> = entries.iter().map(|e| e.panel.clone()).collect();
            let mut location = group_location(&panels, panel)?;
            location.stack = Some((entry.open, entry.height));
            Some(location)
        }
        Node::Split { axis, size, first, second } => {
            let (mut location, sibling, is_first) = if let Some(location) = node_location(first, panel) {
                (location, second, true)
            } else {
                (node_location(second, panel)?, first, false)
            };
            if location.anchor.is_none()
                && let Some(anchor) = first_panel(sibling)
            {
                location.anchor = Some(anchor.clone());
                location.placement = Placement::Split(match (*axis, is_first) {
                    (SplitAxis::Horizontal, true) => Zone::Left,
                    (SplitAxis::Horizontal, false) => Zone::Right,
                    (SplitAxis::Vertical, true) => Zone::Top,
                    (SplitAxis::Vertical, false) => Zone::Bottom,
                });
                location.size = Some(*size);
                let mut members = Vec::new();
                node_members(sibling, &mut members);
                location.sibling = members.into_iter().cloned().collect();
            }
            Some(location)
        }
    }
}

fn restore_parent_size<P: Eq>(node: &mut Node<P>, panel: &P, size: SplitSize) -> bool {
    fn alone<P: Eq>(node: &Node<P>, panel: &P) -> bool {
        matches!(node, Node::Tabs { panels, .. } if panels.len() == 1 && panels.first() == Some(panel))
    }
    if let Node::Split { size: current, first, second, .. } = node {
        if alone(first, panel) || alone(second, panel) {
            *current = size;
            true
        } else {
            restore_parent_size(first, panel, size) || restore_parent_size(second, panel, size)
        }
    } else {
        false
    }
}

fn restore_stack_kind<P: Clone + Eq>(node: &mut Node<P>, panel: &P) {
    match node {
        Node::Tabs { panels, .. } if panels.len() == 1 && panels.first() == Some(panel) => {
            *node = Node::Stack { entries: vec![StackEntry { panel: panel.clone(), open: true, height: None }] };
        }
        Node::Split { first, second, .. } => {
            restore_stack_kind(first, panel);
            restore_stack_kind(second, panel);
        }
        _ => {}
    }
}

fn node_members<'a, P>(node: &'a Node<P>, members: &mut Vec<&'a P>) {
    match node {
        Node::Tabs { panels, .. } => members.extend(panels),
        Node::Stack { entries } => members.extend(entries.iter().map(|e| &e.panel)),
        Node::Split { first, second, .. } => {
            node_members(first, members);
            node_members(second, members);
        }
    }
}

fn restore_around_subtree<P: Clone + Eq + Hash>(node: &mut Node<P>, panel: &P, members: &HashSet<&P>, zone: Zone, size: SplitSize) -> bool {
    let mut actual = Vec::new();
    node_members(node, &mut actual);
    if actual.len() == members.len() && actual.iter().all(|p| members.contains(p)) {
        let new = Node::Tabs { panels: vec![panel.clone()], active: 0 };
        let old = std::mem::replace(node, Node::Tabs { panels: Vec::new(), active: 0 });
        let (first, second) = if matches!(zone, Zone::Left | Zone::Top) { (new, old) } else { (old, new) };
        *node = Node::Split {
            axis: if matches!(zone, Zone::Top | Zone::Bottom) { SplitAxis::Vertical } else { SplitAxis::Horizontal },
            size,
            first: Box::new(first),
            second: Box::new(second),
        };
        true
    } else if let Node::Split { first, second, .. } = node {
        restore_around_subtree(first, panel, members, zone, size) || restore_around_subtree(second, panel, members, zone, size)
    } else {
        false
    }
}
