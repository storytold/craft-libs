//! The private transaction steps: apply one action to a proposed copy, and the helpers that
//! check, remove and insert panels in a tree.

use std::{collections::HashSet, hash::Hash};

use crate::flow::Axis;

use super::{
    DockSide,
    action::{Action, DockPlacement, Location},
    edit::{node_at, split_frame, split_frame_sized},
    groups::{Column, DockEdge, Group, Moving, PanelPlace, Target, Unit},
    layout::Layout,
    limits::{MAX_DEPTH, MAX_PANELS, valid_active, valid_height, valid_rect, valid_size, valid_width},
    node::{Floating, Node, StackEntry},
    policy::{DockError, Permissions},
    split::SplitSize,
};

impl<P: Clone + Eq + Hash> Layout<P> {
    /// `canvas` is the protected-panel test an outer edge drop narrows by.
    pub(super) fn apply_inner(&mut self, action: Action<P>, canvas: &dyn Fn(&P) -> bool) -> Result<(), DockError> {
        match action {
            Action::Move { panel, anchor, placement } => {
                if panel == anchor {
                    return Err(DockError::InvalidPlacement);
                }
                if !self.contains(&anchor) {
                    return Err(DockError::MissingTarget);
                }
                if let DockPlacement::Tab { before: Some(before) } = &placement {
                    if before == &panel {
                        // A tab dropped immediately before itself has not moved.
                        return if self.same_group(&panel, &anchor) { Ok(()) } else { Err(DockError::InvalidPlacement) };
                    }
                    if !self.same_group(before, &anchor) {
                        return Err(DockError::MissingTarget);
                    }
                }
                let entry = self.take(&panel).ok_or(DockError::MissingPanel)?;
                self.insert(entry, &anchor, placement, canvas)
            }
            Action::Float { panel, rect } => {
                if !valid_rect(rect) {
                    return Err(DockError::InvalidGeometry);
                }
                // A window's only panel keeps the window's id.
                let lone = self.floating.iter().find(|w| w.contains(&panel)).filter(|w| w.panels().count() == 1).map(|w| w.id);
                let entry = self.take(&panel).ok_or(DockError::MissingPanel)?;
                let window = Floating::single(vec![entry.panel], 0, rect);
                self.floating.push(Floating { id: lone.unwrap_or(window.id), ..window });
                Ok(())
            }
            Action::Close { panel } => self.take(&panel).map(|_| ()).ok_or(DockError::MissingPanel),
            Action::Activate { panel } => {
                let place = self.place(&panel).ok_or(DockError::MissingPanel)?;
                if let Some(group) = self.group_mut(&place) {
                    // Selecting a panel shows it: a minimized group expands.
                    group.active = group.panels.iter().position(|p| p == &panel).ok_or(DockError::MissingPanel)?;
                    group.minimized = false;
                    return Ok(());
                }
                if let PanelPlace::Floating { window, .. } = place
                    && let Some(window) = self.floating.get_mut(window)
                {
                    window.minimized = false;
                }
                if self.trees_mut().any(|node| activate(node, &panel)) {
                    return Ok(());
                }
                Err(DockError::MissingPanel)
            }
            Action::Open { panel, anchor } => {
                if self.contains(&panel) {
                    return self.apply_inner(Action::Activate { panel }, canvas);
                }
                let entry = StackEntry { panel, open: true, height: None };
                if let Some(anchor) = anchor {
                    self.insert(entry, &anchor, DockPlacement::Tab { before: None }, canvas)
                } else if self.root.is_none() {
                    self.root = Some(Node::Tabs { panels: vec![entry.panel], active: 0 });
                    Ok(())
                } else {
                    Err(DockError::MissingTarget)
                }
            }
            Action::OpenAt { panel, anchor, placement } => {
                if self.contains(&panel) {
                    return self.apply_inner(Action::Move { panel, anchor, placement }, canvas);
                }
                if let DockPlacement::Tab { before: Some(before) } = &placement
                    && !self.same_group(before, &anchor)
                {
                    return Err(DockError::MissingTarget);
                }
                self.insert(StackEntry { panel, open: true, height: None }, &anchor, placement, canvas)
            }
            Action::MoveFloating { panel, rect } => {
                if !valid_rect(rect) {
                    return Err(DockError::InvalidGeometry);
                }
                let window = self.floating.iter_mut().find(|g| g.contains(&panel)).ok_or(DockError::MissingPanel)?;
                window.rect = rect;
                Ok(())
            }
            Action::ResizeSplit { path, size } => {
                if path.len() >= MAX_DEPTH {
                    return Err(DockError::LimitExceeded);
                }
                if !valid_size(size) {
                    return Err(DockError::InvalidGeometry);
                }
                resize_split(self.root.as_mut().ok_or(DockError::MissingTarget)?, &path, size)
            }
            Action::ResizeWindowSplit { panel, path, size } => {
                if path.len() >= MAX_DEPTH {
                    return Err(DockError::LimitExceeded);
                }
                if !valid_size(size) {
                    return Err(DockError::InvalidGeometry);
                }
                let window = self.floating.iter_mut().find(|w| w.contains(&panel)).ok_or(DockError::MissingPanel)?;
                resize_split(&mut window.root, &path, size)
            }
            Action::SetStackOpen { panel, open } => {
                let entry = self.trees_mut().find_map(|n| stack_entry_mut(n, &panel)).ok_or(DockError::MissingPanel)?;
                entry.open = open;
                Ok(())
            }
            Action::ResizeStack { panel, height } => {
                if !valid_height(height) {
                    return Err(DockError::InvalidGeometry);
                }
                let entry = self.trees_mut().find_map(|n| stack_entry_mut(n, &panel)).ok_or(DockError::MissingPanel)?;
                entry.height = height;
                Ok(())
            }
            Action::MoveGroup { ref panel, ref to } | Action::MoveColumn { ref panel, ref to } | Action::MoveWindow { ref panel, ref to } => {
                let unit = unit_of(&action);
                if let Target::At { anchor, .. } = to
                    && self.members(panel, unit)?.contains(anchor)
                {
                    // Dropped on itself.
                    return Ok(());
                }
                let moving = self.take_unit(panel, unit)?;
                self.insert_moving(moving, to.clone(), canvas)
            }
            Action::StepGroup { panel, up } => {
                let (anchor, placement) = self.step_target(&panel, up, canvas)?;
                self.apply_inner(Action::MoveGroup { panel, to: Target::At { anchor, placement } }, canvas)
            }
            Action::MovePanel { panel, to } => match to {
                Target::At { anchor, placement } => self.apply_inner(Action::Move { panel, anchor, placement }, canvas),
                to => {
                    let entry = self.take(&panel).ok_or(DockError::MissingPanel)?;
                    self.insert_moving(Moving::group(Group::new(vec![entry.panel], 0)), to, canvas)
                }
            },
            Action::CloseGroup { ref panel } | Action::CloseWindow { ref panel } => self.take_unit(panel, unit_of(&action)).map(|_| ()),
            Action::CloseOthers { panel } => {
                for other in self.members(&panel, Unit::Group)? {
                    if other != panel {
                        self.take(&other).ok_or(DockError::MissingPanel)?;
                    }
                }
                Ok(())
            }
            Action::SetMinimized { panel, minimized } => {
                let place = self.place(&panel).ok_or(DockError::MissingPanel)?;
                if let Some(group) = self.group_mut(&place) {
                    group.minimized = minimized;
                    return Ok(());
                }
                if let PanelPlace::Floating { window, .. } = place {
                    self.floating.get_mut(window).ok_or(DockError::MissingPanel)?.minimized = minimized;
                    return Ok(());
                }
                let entry = self.trees_mut().find_map(|n| stack_entry_mut(n, &panel)).ok_or(DockError::InvalidPlacement)?;
                entry.open = !minimized;
                Ok(())
            }
            Action::ResizeGroup { panel, height } => {
                if !valid_height(height) {
                    return Err(DockError::InvalidGeometry);
                }
                let place = self.place(&panel).ok_or(DockError::MissingPanel)?;
                if let Some(group) = self.group_mut(&place) {
                    group.height = height;
                    return Ok(());
                }
                let entry = self.trees_mut().find_map(|n| stack_entry_mut(n, &panel)).ok_or(DockError::InvalidPlacement)?;
                entry.height = height;
                Ok(())
            }
            Action::ResizeFlyoutWidth { panel, width } => {
                if width.is_some_and(|w| !valid_width(w)) {
                    return Err(DockError::InvalidGeometry);
                }
                let place = self.place(&panel).ok_or(DockError::MissingPanel)?;
                self.group_mut(&place).ok_or(DockError::InvalidPlacement)?.flyout_width = width;
                Ok(())
            }
            Action::SetIconic { panel, iconic } => {
                self.column_of(&panel)?.iconic = iconic;
                Ok(())
            }
            Action::ResizeColumn { panel, width } => {
                if !valid_width(width) {
                    return Err(DockError::InvalidGeometry);
                }
                self.column_of(&panel)?.width = Some(width);
                Ok(())
            }
            Action::ResizeIconic { panel, width } => {
                if width.is_some_and(|w| !valid_width(w)) {
                    return Err(DockError::InvalidGeometry);
                }
                self.column_of(&panel)?.iconic_width = width;
                Ok(())
            }
            Action::SetNative { panel, placement } => {
                let placement = match placement {
                    Some(placement) => Some(placement.repaired().ok_or(DockError::InvalidGeometry)?),
                    None => None,
                };
                let window = self.floating.iter_mut().find(|g| g.contains(&panel)).ok_or(DockError::InvalidPlacement)?;
                window.native = placement;
                Ok(())
            }
        }
    }

    pub(super) fn column_of(&mut self, panel: &P) -> Result<&mut Column<P>, DockError> {
        let place = self.place(panel).ok_or(DockError::MissingPanel)?;
        self.column_mut(&place).ok_or(DockError::InvalidPlacement)
    }

    pub(super) fn same_group(&self, panel: &P, anchor: &P) -> bool {
        let place = self.place(panel);
        place.is_some() && place == self.place(anchor)
    }

    pub(super) fn take(&mut self, panel: &P) -> Option<StackEntry<P>> {
        let mut removed = None;
        self.root = self.root.take().and_then(|n| remove_node(n, panel, &mut removed));
        if removed.is_some() {
            return removed;
        }
        if let Some(index) = self.floating.iter().position(|w| w.contains(panel))
            && let Some(window) = self.floating.get_mut(index)
        {
            let root = std::mem::replace(&mut window.root, Node::Tabs { panels: Vec::new(), active: 0 });
            match remove_node(root, panel, &mut removed) {
                Some(root) => window.root = root,
                None => {
                    self.floating.remove(index);
                }
            }
        }
        removed
    }

    /// Where [`Action::Open`] without an anchor puts an absent panel in a layout with a root:
    /// the end of the first shown dock group that accepts tabs, in the end-side dock if one has
    /// such a group, else a new frame at the end edge of the docked area.
    pub(super) fn open_place(&self, permissions: impl Fn(&P) -> Permissions) -> Option<(P, DockPlacement<P>)> {
        let docked: Vec<(&P, PanelPlace)> = self
            .panels()
            .into_iter()
            .filter_map(|p| Some((p, self.place(p)?)))
            .filter(|(_, place)| !matches!(place, PanelPlace::Floating { .. }))
            .collect();
        let side = |place: &PanelPlace| match place {
            PanelPlace::Dock { path, .. } => match self.root.as_ref().and_then(|root| node_at(root, path)) {
                Some(Node::Dock { side, .. }) => Some(*side),
                _ => None,
            },
            _ => None,
        };
        // A minimized group or an iconic column would leave the panel open but not shown.
        let shown = |place: &PanelPlace| match place {
            PanelPlace::Dock { group, .. } => self.column(place).is_some_and(|c| !c.iconic && c.groups.get(*group).is_some_and(|g| !g.minimized)),
            _ => false,
        };
        let groups = docked.iter().filter(|(p, place)| side(place).is_some() && shown(place) && permissions(p).accepts_tabs);
        if let Some((panel, _)) = groups.clone().find(|(_, place)| side(place) == Some(DockEdge::End)).or_else(|| groups.clone().next()) {
            return Some(((*panel).clone(), DockPlacement::Tab { before: None }));
        }
        // A canvas anchor would narrow the edge to the canvas column.
        let edge = docked.iter().find(|(p, _)| !permissions(p).is_protected()).or(docked.first());
        edge.map(|(panel, _)| ((*panel).clone(), DockPlacement::Edge(DockSide::End)))
    }

    pub(super) fn insert(
        &mut self,
        entry: StackEntry<P>,
        anchor: &P,
        placement: DockPlacement<P>,
        canvas: &dyn Fn(&P) -> bool,
    ) -> Result<(), DockError> {
        let place = self.place(anchor).ok_or(DockError::MissingTarget)?;
        let tree = place.tree_path().map(|(window, path)| (window, path.to_vec()));
        let Some((window, path)) =
            tree.filter(|_| !matches!(placement, DockPlacement::Group { .. } | DockPlacement::Column { .. } | DockPlacement::Edge(_)))
        else {
            let group = Group::new(vec![entry.panel], 0);
            return self.insert_moving(Moving::group(group), Target::At { anchor: anchor.clone(), placement }, canvas);
        };
        let root = self.tree_mut(window).ok_or(DockError::MissingTarget)?;
        if let DockPlacement::Split(zone) = placement
            && zone != DockSide::Center
        {
            let new = Node::Tabs { panels: vec![entry.panel], active: 0 };
            return if split_frame(root, &path, new, zone) { Ok(()) } else { Err(DockError::MissingTarget) };
        }
        if let DockPlacement::SplitSized { zone, size, outer } = placement {
            let new = Node::Tabs { panels: vec![entry.panel], active: 0 };
            return if split_frame_sized(root, &path, new, zone, size, outer) { Ok(()) } else { Err(DockError::InvalidPlacement) };
        }
        if insert_node(root, &entry, anchor, &placement) { Ok(()) } else { Err(DockError::MissingTarget) }
    }
}

pub(super) fn unit_of<P>(action: &Action<P>) -> Unit {
    match action {
        Action::MoveColumn { .. } => Unit::Column,
        Action::MoveWindow { .. } | Action::CloseWindow { .. } => Unit::Window,
        _ => Unit::Group,
    }
}

pub(super) fn check_panel<'a, P: Eq + Hash>(panel: &'a P, seen: &mut HashSet<&'a P>) -> Result<(), DockError> {
    if seen.len() >= MAX_PANELS {
        return Err(DockError::LimitExceeded);
    }
    if !seen.insert(panel) {
        return Err(DockError::DuplicatePanel);
    }
    Ok(())
}

/// A `Tabs` or `Stack` node: not empty, a valid selection and heights, panels not seen before.
pub(super) fn check_leaf<'a, P: Eq + Hash>(node: &'a Node<P>, seen: &mut HashSet<&'a P>) -> Result<(), DockError> {
    match node {
        Node::Tabs { panels, active } => {
            if panels.is_empty() {
                return Err(DockError::InvalidGeometry);
            }
            check_tabs(panels, *active, seen)
        }
        Node::Stack { entries } => {
            if entries.is_empty() {
                return Err(DockError::InvalidGeometry);
            }
            if entries.len() > MAX_PANELS {
                return Err(DockError::LimitExceeded);
            }
            for entry in entries {
                if !valid_height(entry.height) {
                    return Err(DockError::InvalidGeometry);
                }
                check_panel(&entry.panel, seen)?;
            }
            Ok(())
        }
        Node::Split { .. } | Node::Dock { .. } => Ok(()),
    }
}

pub(super) fn check_group<'a, P: Eq + Hash>(group: &'a Group<P>, seen: &mut HashSet<&'a P>) -> Result<(), DockError> {
    if group.panels.is_empty() || !valid_height(group.height) || group.flyout_width.is_some_and(|w| !valid_width(w)) {
        return Err(DockError::InvalidGeometry);
    }
    check_tabs(&group.panels, group.active, seen)
}

pub(super) fn check_tabs<'a, P: Eq + Hash>(panels: &'a [P], active: usize, seen: &mut HashSet<&'a P>) -> Result<(), DockError> {
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

pub(super) fn remove_tab<P: Clone + Eq>(panels: &mut Vec<P>, active: &mut usize, panel: &P) -> Option<P> {
    let index = panels.iter().position(|p| p == panel)?;
    let selected = panels.get(*active).cloned();
    let removed = panels.remove(index);
    *active = selected.as_ref().and_then(|p| panels.iter().position(|q| q == p)).unwrap_or(index.min(panels.len().saturating_sub(1)));
    Some(removed)
}

pub(super) fn remove_node<P: Clone + Eq>(node: Node<P>, panel: &P, removed: &mut Option<StackEntry<P>>) -> Option<Node<P>> {
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
        Node::Dock { side, mut columns } => {
            // A dock that was already empty stays; one emptied by this removal collapses.
            let had_columns = !columns.is_empty();
            for column in &mut columns {
                for group in &mut column.groups {
                    if let Some(panel) = remove_tab(&mut group.panels, &mut group.active, panel) {
                        *removed = Some(StackEntry { panel, open: true, height: None });
                    }
                }
                column.groups.retain(|g| !g.panels.is_empty());
            }
            columns.retain(|c| !c.groups.is_empty());
            (!had_columns || !columns.is_empty()).then_some(Node::Dock { side, columns })
        }
    }
}

pub(super) fn resize_split<P>(mut node: &mut Node<P>, path: &[bool], size: SplitSize) -> Result<(), DockError> {
    for second_child in path {
        let Node::Split { first, second, .. } = node else { return Err(DockError::MissingTarget) };
        node = if *second_child { second } else { first };
    }
    let Node::Split { size: old, .. } = node else { return Err(DockError::MissingTarget) };
    *old = size;
    Ok(())
}

pub(super) fn activate<P: Eq>(node: &mut Node<P>, panel: &P) -> bool {
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
        Node::Dock { .. } => false,
    }
}

pub(super) fn stack_entry_mut<'a, P: Eq>(node: &'a mut Node<P>, panel: &P) -> Option<&'a mut StackEntry<P>> {
    match node {
        Node::Stack { entries } => entries.iter_mut().find(|e| &e.panel == panel),
        Node::Split { first, second, .. } => stack_entry_mut(first, panel).or_else(|| stack_entry_mut(second, panel)),
        Node::Tabs { .. } | Node::Dock { .. } => None,
    }
}

pub(super) fn insert_node<P: Clone + Eq>(node: &mut Node<P>, entry: &StackEntry<P>, anchor: &P, placement: &DockPlacement<P>) -> bool {
    let here = match node {
        Node::Split { first, second, .. } => return insert_node(first, entry, anchor, placement) || insert_node(second, entry, anchor, placement),
        Node::Tabs { panels, .. } => panels.contains(anchor),
        Node::Stack { entries } => entries.iter().any(|e| &e.panel == anchor),
        Node::Dock { .. } => false,
    };
    if !here {
        return false;
    }
    match node {
        Node::Tabs { panels, active } => {
            let index = match placement {
                DockPlacement::Tab { before } => before.as_ref().and_then(|p| panels.iter().position(|q| q == p)).unwrap_or(panels.len()),
                _ => panels.iter().position(|p| p == anchor).map_or(panels.len(), |i| i + 1),
            };
            panels.insert(index, entry.panel.clone());
            *active = index;
        }
        Node::Stack { entries } => {
            let index = match placement {
                DockPlacement::Tab { before } => before.as_ref().and_then(|p| entries.iter().position(|e| &e.panel == p)).unwrap_or(entries.len()),
                _ => entries.iter().position(|e| &e.panel == anchor).map_or(entries.len(), |i| i + 1),
            };
            entries.insert(index, entry.clone());
        }
        Node::Split { .. } | Node::Dock { .. } => return false,
    }
    true
}

pub(super) fn group_location<P: Clone + Eq>(panels: &[P], panel: &P) -> Option<Location<P>> {
    let index = panels.iter().position(|p| p == panel)?;
    let anchor = panels.iter().find(|p| *p != panel).cloned();
    let before = panels.get(index + 1).cloned();
    Some(Location { floating: None, anchor, placement: DockPlacement::Tab { before }, size: None, stack: None, sibling: Vec::new(), group: None })
}

pub(super) fn first_panel<P>(node: &Node<P>) -> Option<&P> {
    match node {
        Node::Tabs { panels, .. } => panels.first(),
        Node::Stack { entries } => entries.first().map(|e| &e.panel),
        Node::Split { first, second, .. } => first_panel(first).or_else(|| first_panel(second)),
        Node::Dock { columns, .. } => columns.iter().flat_map(|c| c.groups.iter()).find_map(|g| g.panels.first()),
    }
}

pub(super) fn node_location<P: Clone + Eq>(node: &Node<P>, panel: &P) -> Option<Location<P>> {
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
                location.placement = DockPlacement::Split(match (*axis, is_first) {
                    (Axis::X, true) => DockSide::Start,
                    (Axis::X, false) => DockSide::End,
                    (Axis::Y, true) => DockSide::Top,
                    (Axis::Y, false) => DockSide::Bottom,
                });
                location.size = Some(*size);
                let mut members = Vec::new();
                node_members(sibling, &mut members);
                location.sibling = members.into_iter().cloned().collect();
            }
            Some(location)
        }
        Node::Dock { .. } => None,
    }
}

pub(super) fn restore_parent_size<P: Eq>(node: &mut Node<P>, panel: &P, size: SplitSize) -> bool {
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

pub(super) fn restore_stack_kind<P: Clone + Eq>(node: &mut Node<P>, panel: &P) {
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

pub(super) fn node_members<'a, P>(node: &'a Node<P>, members: &mut Vec<&'a P>) {
    match node {
        Node::Tabs { panels, .. } => members.extend(panels),
        Node::Stack { entries } => members.extend(entries.iter().map(|e| &e.panel)),
        Node::Split { first, second, .. } => {
            node_members(first, members);
            node_members(second, members);
        }
        Node::Dock { columns, .. } => members.extend(columns.iter().flat_map(|c| c.groups.iter()).flat_map(|g| g.panels.iter())),
    }
}

pub(super) fn restore_around_subtree<P: Clone + Eq + Hash>(
    node: &mut Node<P>,
    panel: &P,
    members: &HashSet<&P>,
    zone: DockSide,
    size: SplitSize,
) -> bool {
    let mut actual = Vec::new();
    node_members(node, &mut actual);
    if actual.len() == members.len() && actual.iter().all(|p| members.contains(p)) {
        let new = Node::Tabs { panels: vec![panel.clone()], active: 0 };
        let old = std::mem::replace(node, Node::Tabs { panels: Vec::new(), active: 0 });
        let (first, second) = if matches!(zone, DockSide::Start | DockSide::Top) { (new, old) } else { (old, new) };
        *node = Node::Split {
            axis: if matches!(zone, DockSide::Top | DockSide::Bottom) { Axis::Y } else { Axis::X },
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
