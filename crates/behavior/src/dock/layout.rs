//! The layout: a docked tree plus floating windows, with validation, queries and restore.

use std::{collections::HashSet, hash::Hash};

use crate::flow::Axis;

use super::{
    DockSide,
    action::{Action, DockPlacement, Location},
    edit::{node_at, valid_column},
    groups::{Column, ColumnLocation, DockEdge, Group, GroupLocation, Moving, PanelPlace, PanelState, Target},
    inner::{
        check_group, check_leaf, group_location, node_location, restore_around_subtree, restore_parent_size, restore_stack_kind, stack_entry_mut,
    },
    limits::{MAX_DEPTH, MAX_NODES, MAX_PANELS, valid_height, valid_rect, valid_size, valid_width},
    node::{Floating, Node},
    policy::{DockError, Permissions},
    split::SplitSize,
};

/// The panels of a workspace: a docked tree plus floating windows. Every panel occurs once.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Layout<P> {
    /// The docked layout; `None` when nothing is docked.
    pub root: Option<Node<P>>,
    /// Floating windows, with unique ids.
    #[serde(deserialize_with = "super::node::floating_serde::unique_ids")]
    pub floating: Vec<Floating<P>>,
}

impl<P> Default for Layout<P> {
    fn default() -> Self {
        Self { root: None, floating: Vec::new() }
    }
}

impl<P> Layout<P> {
    /// A layout with the docked tree `root` and the floating windows `floating`.
    pub fn new(root: Option<Node<P>>, floating: Vec<Floating<P>>) -> Self {
        Self { root, floating }
    }
}

impl<P: Clone + Eq + Hash> Layout<P> {
    /// Check depth, allocation bounds, geometry, selection, globally unique panel ownership and
    /// that no frame, group, column or window is empty.
    /// Traversal is iterative, including for an invalid, excessively deep supplied tree.
    pub fn validate(&self) -> Result<(), DockError> {
        if self.floating.len() > MAX_PANELS {
            return Err(DockError::LimitExceeded);
        }
        let mut seen = HashSet::new();
        let mut count = 0usize;
        let mut docks = (false, false);
        // Each node carries whether it touches the window's start and end edges.
        let mut visit = self.root.as_ref().map(|n| vec![(n, 1usize, true, true)]).unwrap_or_default();
        while let Some((node, depth, start, end)) = visit.pop() {
            count = count.saturating_add(1);
            if depth > MAX_DEPTH || count > MAX_NODES {
                return Err(DockError::LimitExceeded);
            }
            match node {
                Node::Split { axis, size, first, second } => {
                    if !valid_size(*size) {
                        return Err(DockError::InvalidGeometry);
                    }
                    let horizontal = *axis == Axis::X;
                    visit.push((second, depth + 1, false, end && horizontal));
                    visit.push((first, depth + 1, start && horizontal, false));
                }
                Node::Dock { side, columns } => {
                    let (at_edge, taken) = match side {
                        DockEdge::Start => (start, &mut docks.0),
                        DockEdge::End => (end, &mut docks.1),
                    };
                    if !at_edge || *taken {
                        return Err(DockError::InvalidPlacement);
                    }
                    *taken = true;
                    if columns.len() > MAX_PANELS {
                        return Err(DockError::LimitExceeded);
                    }
                    for column in columns {
                        if !valid_column(column) || column.groups.is_empty() {
                            return Err(DockError::InvalidGeometry);
                        }
                        count = count.saturating_add(column.groups.len().saturating_add(1));
                        if count > MAX_NODES {
                            return Err(DockError::LimitExceeded);
                        }
                        for group in &column.groups {
                            check_group(group, &mut seen)?;
                        }
                    }
                }
                Node::Tabs { .. } | Node::Stack { .. } => check_leaf(node, &mut seen)?,
            }
        }
        for floating in &self.floating {
            if !valid_rect(floating.rect) {
                return Err(DockError::InvalidGeometry);
            }
            let mut visit = vec![(&floating.root, 1usize)];
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
                    Node::Dock { .. } => return Err(DockError::InvalidPlacement),
                    Node::Tabs { .. } | Node::Stack { .. } => check_leaf(node, &mut seen)?,
                }
            }
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
                Node::Dock { columns, .. } => {
                    let members = columns.iter().flat_map(|c| c.groups.iter()).flat_map(|g| g.panels.iter());
                    panels.extend(members.take(MAX_PANELS - panels.len()));
                }
            }
        }
        for window in self.floating.iter().take(MAX_PANELS) {
            panels.extend(window.panels().take(MAX_PANELS - panels.len()));
        }
        panels
    }

    /// Whether the layout holds `panel`, docked or floating.
    pub fn contains(&self, panel: &P) -> bool {
        self.panels().contains(&panel)
    }

    /// The panels `panel` can join as a tab: every panel in another group whose permissions
    /// accept tabs. A "Move Panel To" menu with an empty answer has nothing to offer and
    /// stays hidden.
    pub fn move_targets(&self, panel: &P, permissions: impl Fn(&P) -> Permissions) -> Vec<&P> {
        let own = self.place(panel);
        self.panels()
            .into_iter()
            .filter(|target| *target != panel && permissions(target).accepts_tabs && (own.is_none() || self.place(target) != own))
            .collect()
    }

    /// Whether [`Action::StepGroup`] would move `panel`'s group, for enabling a menu item.
    pub fn can_step_group(&self, panel: &P, up: bool, permissions: impl Fn(&P) -> Permissions) -> bool {
        self.step_target(panel, up, &|p| {
            let policy = permissions(p);
            !policy.movable || policy.is_protected()
        })
        .is_ok()
    }

    /// The anchor and placement that move `panel`'s dock group past its neighbour in the column.
    /// `pinned` panels stay put: the group and its neighbour must hold none.
    pub(super) fn step_target(&self, panel: &P, up: bool, pinned: &dyn Fn(&P) -> bool) -> Result<(P, DockPlacement<P>), DockError> {
        let place = self.place(panel).ok_or(DockError::MissingPanel)?;
        let PanelPlace::Dock { group, .. } = place else { return Err(DockError::InvalidPlacement) };
        let groups = &self.column(&place).ok_or(DockError::MissingPanel)?.groups;
        let neighbour = if up { group.checked_sub(1) } else { group.checked_add(1) }.and_then(|i| groups.get(i)).ok_or(DockError::MissingTarget)?;
        let own = groups.get(group).ok_or(DockError::MissingPanel)?;
        if own.panels.iter().chain(&neighbour.panels).any(pinned) {
            return Err(DockError::NotAllowed);
        }
        let anchor = neighbour.panels.first().ok_or(DockError::MissingTarget)?.clone();
        Ok((anchor, DockPlacement::Group { below: !up, height: None }))
    }

    /// Capture a panel's nearest surviving group/split neighbor before hiding it.
    pub fn location(&self, panel: &P) -> Result<Location<P>, DockError> {
        self.validate()?;
        let place = self.place(panel).ok_or(DockError::MissingPanel)?;
        let (groups, group, column) = match &place {
            PanelPlace::Node(_) => return self.root.as_ref().and_then(|node| node_location(node, panel)).ok_or(DockError::MissingPanel),
            PanelPlace::Floating { window, .. } => {
                let window = self.floating.get(*window).ok_or(DockError::MissingPanel)?;
                let mut location = node_location(&window.root, panel).ok_or(DockError::MissingPanel)?;
                location.floating = Some(window.rect);
                return Ok(location);
            }
            PanelPlace::Dock { path, group, .. } => {
                let side = match self.root.as_ref().and_then(|root| node_at(root, path)) {
                    Some(Node::Dock { side, .. }) => *side,
                    _ => return Err(DockError::MissingPanel),
                };
                let column = self.column(&place).ok_or(DockError::MissingPanel)?;
                let state = ColumnLocation { side, width: column.width, iconic: column.iconic, iconic_width: column.iconic_width };
                (&column.groups, *group, Some(state))
            }
        };
        let own = groups.get(group).ok_or(DockError::MissingPanel)?;
        let mut location = group_location(&own.panels, panel).ok_or(DockError::MissingPanel)?;
        location.group = Some(GroupLocation { minimized: own.minimized, height: own.height, flyout_width: own.flyout_width, column });
        if location.anchor.is_some() {
            return Ok(location);
        }
        // Alone in its group: anchor to the neighboring group, then the neighboring column.
        let first = |g: Option<&Group<P>>| g.and_then(|g| g.panels.first()).cloned();
        if let Some(anchor) = first(groups.get(group + 1)) {
            location.anchor = Some(anchor);
            location.placement = DockPlacement::Group { below: false, height: None };
        } else if let Some(anchor) = first(group.checked_sub(1).and_then(|i| groups.get(i))) {
            location.anchor = Some(anchor);
            location.placement = DockPlacement::Group { below: true, height: None };
        } else if groups.len() == 1
            && let PanelPlace::Dock { path, column, .. } = &place
            && let Some(Node::Dock { columns, .. }) = self.root.as_ref().and_then(|root| node_at(root, path))
        {
            let first_of = |c: Option<&Column<P>>| first(c.and_then(|c| c.groups.first()));
            if let Some(anchor) = first_of(columns.get(column + 1)) {
                location.anchor = Some(anchor);
                location.placement = DockPlacement::Column { after: false };
            } else if let Some(anchor) = first_of(column.checked_sub(1).and_then(|i| columns.get(i))) {
                location.anchor = Some(anchor);
                location.placement = DockPlacement::Column { after: true };
            }
        }
        Ok(location)
    }

    /// Hidden, open behind another tab or in a collapsed container, or showing.
    pub fn panel_state(&self, panel: &P) -> PanelState {
        let showing = match self.place(panel) {
            None => return PanelState::Hidden,
            Some(place @ (PanelPlace::Node(_) | PanelPlace::Floating { .. })) => {
                let minimized = matches!(place, PanelPlace::Floating { window, .. } if self.floating.get(window).is_some_and(|w| w.minimized));
                !minimized
                    && match self.node_of(&place) {
                        Some(Node::Tabs { panels, active }) => panels.get(*active) == Some(panel),
                        Some(Node::Stack { entries }) => entries.iter().any(|e| &e.panel == panel && e.open),
                        _ => false,
                    }
            }
            Some(place) => {
                let iconic = self.column(&place).is_some_and(|c| c.iconic);
                self.group(&place).is_some_and(|g| !g.minimized && g.active_panel() == Some(panel)) && !iconic
            }
        };
        if showing { PanelState::Front } else { PanelState::Open }
    }

    /// Restore an absent panel at a captured location. Missing anchors are reported without
    /// changes, allowing the app to choose an explicit default; a floating panel whose
    /// window is gone floats at its last rect instead. A tab whose next tab is not in the
    /// anchor's group goes last. Unrelated tabs retain selection.
    pub fn restore(&mut self, panel: P, location: &Location<P>) -> Result<(), DockError> {
        self.validate()?;
        if self.contains(&panel) {
            return Err(DockError::DuplicatePanel);
        }
        if location.size.is_some_and(|s| !valid_size(s)) || location.stack.is_some_and(|(_, h)| !valid_height(h)) {
            return Err(DockError::InvalidGeometry);
        }
        let state = location.group.as_ref();
        if state.is_some_and(|s| {
            !valid_height(s.height)
                || s.flyout_width.is_some_and(|w| !valid_width(w))
                || s.column.is_some_and(|c| c.width.is_some_and(|w| !valid_width(w)) || c.iconic_width.is_some_and(|w| !valid_width(w)))
        }) {
            return Err(DockError::InvalidGeometry);
        }
        // A restore that creates a group, column or dock gives it the saved state.
        let moving = || {
            let group = Group {
                panels: vec![panel.clone()],
                active: 0,
                minimized: state.is_some_and(|s| s.minimized),
                height: state.and_then(|s| s.height),
                flyout_width: state.and_then(|s| s.flyout_width),
            };
            let column = state.and_then(|s| s.column);
            Moving {
                width: column.and_then(|c| c.width),
                iconic: column.is_some_and(|c| c.iconic),
                iconic_width: column.and_then(|c| c.iconic_width),
                ..Moving::group(group)
            }
        };
        let mut proposed = self.clone();
        if location.floating.is_some_and(|rect| !valid_rect(rect)) {
            return Err(DockError::InvalidGeometry);
        }
        let window_open = location.anchor.as_ref().is_some_and(|anchor| proposed.floating.iter().any(|w| w.contains(anchor)));
        if let Some(rect) = location.floating.filter(|_| !window_open) {
            // A window that closed as a whole reopens at its last rect.
            proposed.insert_moving(moving(), Target::Float { rect }, &|_| false)?;
        } else if let Some(anchor) = &location.anchor
            && matches!(location.placement, DockPlacement::Group { .. } | DockPlacement::Column { .. })
        {
            if !proposed.contains(anchor) {
                return Err(DockError::MissingTarget);
            }
            proposed.insert_moving(moving(), Target::At { anchor: anchor.clone(), placement: location.placement.clone() }, &|_| false)?;
        } else if let Some(column) = state.and_then(|s| s.column).filter(|_| location.anchor.is_none()) {
            // Alone in its dock: back at that edge, as the outermost column if a dock exists.
            match proposed.dock_path(column.side) {
                Some(path) => {
                    let columns = proposed.columns_mut(&path).ok_or(DockError::MissingTarget)?;
                    let Moving { groups, .. } = moving();
                    columns.insert(0, Column { groups, width: column.width, iconic: column.iconic, iconic_width: column.iconic_width });
                }
                None => proposed.insert_moving(moving(), Target::NewDock { side: column.side }, &|_| false)?,
            }
        } else if let Some(anchor) = &location.anchor {
            if !proposed.contains(anchor) {
                return Err(DockError::MissingTarget);
            }
            let mut around = false;
            if let DockPlacement::Split(zone @ (DockSide::Start | DockSide::End | DockSide::Top | DockSide::Bottom)) = location.placement
                && !location.sibling.is_empty()
            {
                if location.sibling.len() > MAX_PANELS {
                    return Err(DockError::LimitExceeded);
                }
                let members: HashSet<&P> = location.sibling.iter().collect();
                if members.len() != location.sibling.len() || !members.contains(anchor) {
                    return Err(DockError::InvalidPlacement);
                }
                let size = location.size.unwrap_or(SplitSize::Ratio(0.5));
                around = proposed.trees_mut().any(|root| restore_around_subtree(root, &panel, &members, zone, size));
            }
            // A subtree that changed since (a migrated or reconciled layout) splits the anchor.
            if !around {
                // The tab that followed may have closed or moved since; the panel then goes last.
                let placement = match &location.placement {
                    DockPlacement::Tab { before: Some(before) } if !proposed.same_group(before, anchor) => DockPlacement::Tab { before: None },
                    placement => placement.clone(),
                };
                proposed.apply_inner(Action::OpenAt { panel: panel.clone(), anchor: anchor.clone(), placement }, &|_| false)?;
                if let Some(size) = location.size {
                    proposed.trees_mut().any(|root| restore_parent_size(root, &panel, size));
                }
            }
        } else if proposed.root.is_none() {
            proposed.root = Some(Node::Tabs { panels: vec![panel.clone()], active: 0 });
        } else {
            return Err(DockError::MissingTarget);
        }
        if let Some((open, height)) = location.stack {
            for root in proposed.trees_mut() {
                restore_stack_kind(root, &panel);
                if let Some(entry) = stack_entry_mut(root, &panel) {
                    entry.open = open;
                    entry.height = height;
                }
            }
        }
        proposed.validate()?;
        *self = proposed;
        Ok(())
    }

    /// Open an absent panel as a frame at the outer edge of the docked area, taking most of it:
    /// the last resort for a panel that must stay shown, such as a protected canvas.
    pub fn open_at_edge(&mut self, panel: P) -> Result<(), DockError> {
        let mut proposed = self.clone();
        if self.root.is_none() {
            proposed.root = Some(Node::Tabs { panels: vec![panel], active: 0 });
        } else {
            let anchor = self
                .panels()
                .into_iter()
                .find(|p| matches!(self.place(p), Some(PanelPlace::Dock { .. } | PanelPlace::Node(_))))
                .cloned()
                .ok_or(DockError::MissingTarget)?;
            proposed.apply_inner(Action::OpenAt { panel: panel.clone(), anchor, placement: DockPlacement::Edge(DockSide::End) }, &|_| false)?;
            if let Some(root) = proposed.root.as_mut() {
                restore_parent_size(root, &panel, SplitSize::Ratio(0.25));
            }
        }
        proposed.validate()?;
        *self = proposed;
        Ok(())
    }
}
