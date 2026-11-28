//! The public transactions of a layout: an action under permissions, atomic, with the panels it closed.

use std::{collections::HashSet, hash::Hash};

use super::{
    action::Action,
    edit::{leaves, tree_panels},
    groups::{Target, Unit},
    inner::unit_of,
    layout::Layout,
    node::Node,
    policy::{DockError, Permissions},
};

impl<P: Clone + Eq + Hash> Layout<P> {
    /// [`Self::apply_with_permissions`] with default permissions.
    pub fn apply(&mut self, action: Action<P>) -> Result<Vec<P>, DockError> {
        self.apply_with_permissions(action, |_| Permissions::default())
    }

    /// Check application policy before making an atomic change, and return the panels the
    /// action closed (see [`Self::closed_by`]) so the application can release their state.
    /// The callback must be stable throughout one call. Registry validation remains with the
    /// application, particularly Open. The column rights ([`Permissions::collapses`],
    /// [`Permissions::shares_column`], [`Permissions::docks_outside_columns`]) are checked on the result: an action
    /// that leaves a panel breaking them, which did not before, is refused, so a drag's dry run refuses it alike.
    pub fn apply_with_permissions(&mut self, action: Action<P>, permissions: impl Fn(&P) -> Permissions) -> Result<Vec<P>, DockError> {
        self.validate()?;
        let action = match action {
            Action::Open { panel, anchor: None } if self.root.is_some() && !self.contains(&panel) => {
                let (anchor, placement) = self.open_place(&panel, &permissions).ok_or(DockError::MissingTarget)?;
                Action::OpenAt { panel, anchor, placement }
            }
            Action::StepGroup { panel, up } => {
                let (anchor, placement) = self.step_target(&panel, up, &|p| {
                    let policy = permissions(p);
                    !policy.movable || policy.is_protected()
                })?;
                Action::MoveGroup { panel, to: Target::At { anchor, placement } }
            }
            action => action,
        };
        // A destination in a floating window takes only floatable panels.
        let floats = |anchor: &P| self.floating.iter().any(|w| w.contains(anchor));
        match &action {
            Action::Move { panel, anchor, placement } | Action::OpenAt { panel, anchor, placement } => {
                // Opening an absent panel moves nothing, so it works in a locked workspace too.
                let opens = matches!(action, Action::OpenAt { .. }) && !self.contains(panel);
                if !opens && !permissions(panel).movable {
                    return Err(DockError::NotAllowed);
                }
                if !placement.allowed_by(permissions(anchor)) || floats(anchor) && !permissions(panel).floatable {
                    return Err(DockError::NotAllowed);
                }
            }
            // Floating an absent panel opens it and moves nothing, so it works in a locked workspace too.
            Action::Float { panel, .. } if self.contains(panel) && (!permissions(panel).floatable || !permissions(panel).movable) => {
                return Err(DockError::NotAllowed);
            }
            Action::Close { panel } if !permissions(panel).closable => return Err(DockError::NotAllowed),
            // The rectangle belongs to the whole floating window, so every panel in it must allow it.
            Action::MoveFloating { panel, .. }
                if !permissions(panel).movable
                    || self.floating.iter().find(|w| w.contains(panel)).is_some_and(|w| w.panels().any(|p| !permissions(p).movable)) =>
            {
                return Err(DockError::NotAllowed);
            }
            Action::Open { anchor: Some(anchor), .. } if !permissions(anchor).accepts_tabs => return Err(DockError::NotAllowed),
            Action::MovePanel { panel, to } => {
                let policy = permissions(panel);
                let allowed = policy.movable
                    && match to {
                        Target::Float { .. } => policy.floatable,
                        Target::At { anchor, placement } => placement.allowed_by(permissions(anchor)) && (!floats(anchor) || policy.floatable),
                        Target::NewDock { .. } => true,
                    };
                if !allowed {
                    return Err(DockError::NotAllowed);
                }
            }
            Action::Restore { panel, location } => {
                let policy = permissions(panel);
                let allowed = policy.movable
                    && location
                        .anchor
                        .as_ref()
                        .is_none_or(|anchor| location.placement.allowed_by(permissions(anchor)) && (!floats(anchor) || policy.floatable))
                    && (location.floating.is_none() || policy.floatable);
                if !allowed {
                    return Err(DockError::NotAllowed);
                }
            }
            Action::MoveGroup { panel, to } | Action::MoveColumn { panel, to } | Action::MoveWindow { panel, to } => {
                let members = self.members(panel, unit_of(&action))?;
                if members.iter().any(|p| !permissions(p).movable) {
                    return Err(DockError::NotAllowed);
                }
                match to {
                    Target::Float { .. } if members.iter().any(|p| !permissions(p).floatable) => return Err(DockError::NotAllowed),
                    Target::At { anchor, placement }
                        if !members.contains(anchor)
                            && (!placement.allowed_by(permissions(anchor))
                                || floats(anchor) && members.iter().any(|p| !permissions(p).floatable)) =>
                    {
                        return Err(DockError::NotAllowed);
                    }
                    _ => {}
                }
            }
            Action::CloseGroup { panel } | Action::CloseWindow { panel } | Action::CloseOthers { panel } => {
                let others = matches!(action, Action::CloseOthers { .. });
                let members = self.members(panel, unit_of(&action))?;
                if members.iter().filter(|p| !others || *p != panel).any(|p| !permissions(p).closable) {
                    return Err(DockError::NotAllowed);
                }
            }
            Action::SetNative { panel, placement: Some(_) } if self.members(panel, Unit::Window)?.iter().any(|p| !permissions(p).floatable) => {
                return Err(DockError::NotAllowed);
            }
            _ => {}
        }
        let closed = self.closed_by(&action)?;
        let mut proposed = self.clone();
        proposed.apply_inner(action, &|p| permissions(p).is_protected())?;
        proposed.validate()?;
        if !proposed.breaks_columns_beyond(self, &permissions) {
            *self = proposed;
            return Ok(closed);
        }
        Err(DockError::NotAllowed)
    }

    /// Whether a panel breaks its column rights in this layout that did not in `before`.
    pub(super) fn breaks_columns_beyond(&self, before: &Self, permissions: &impl Fn(&P) -> Permissions) -> bool {
        let breaches = self.column_breaches(permissions);
        !breaches.is_empty() && !breaches.is_subset(&before.column_breaches(permissions))
    }

    /// The panels that break their column rights: a panel that does not collapse ([`Permissions::collapses`]) in
    /// a collapsed column, every panel of a column (or node of the docked tree) that holds a panel that does
    /// not share it ([`Permissions::shares_column`]) beside another panel, and a panel that docks only in dock
    /// columns ([`Permissions::docks_outside_columns`]) in a frame of the docked tree.
    pub(super) fn column_breaches(&self, permissions: &impl Fn(&P) -> Permissions) -> HashSet<P> {
        let mut breaches = HashSet::new();
        let mut framed = Vec::new();
        let mut check = |panels: Vec<&P>, iconic: bool| {
            if iconic {
                breaches.extend(panels.iter().filter(|p| !permissions(p).collapses).map(|p| (*p).clone()));
            }
            if panels.len() > 1 && panels.iter().any(|p| !permissions(p).shares_column) {
                breaches.extend(panels.into_iter().cloned());
            }
        };
        for window in &self.floating {
            check(window.panels().collect(), window.column.iconic);
        }
        for (leaf, _) in self.root.as_ref().map(leaves).unwrap_or_default() {
            match leaf {
                Node::Dock { columns, .. } => {
                    for column in columns {
                        check(column.panels().collect(), column.iconic);
                    }
                }
                leaf => {
                    let panels = tree_panels(leaf);
                    framed.extend(panels.iter().filter(|p| !permissions(p).docks_outside_columns).map(|p| (*p).clone()));
                    check(panels, false);
                }
            }
        }
        breaches.extend(framed);
        breaches
    }

    /// Apply every action in order, or none: the first rejection leaves the layout unchanged.
    /// Returns every panel the actions closed, in order.
    pub fn apply_all(&mut self, actions: impl IntoIterator<Item = Action<P>>, permissions: impl Fn(&P) -> Permissions) -> Result<Vec<P>, DockError> {
        let mut proposed = self.clone();
        let mut closed = Vec::new();
        for action in actions {
            closed.extend(proposed.apply_with_permissions(action, &permissions)?);
        }
        *self = proposed;
        Ok(closed)
    }

    /// The panels a close action (`Close`, `CloseGroup`, `CloseWindow`, `CloseOthers`) would
    /// remove, in group order. Empty for other actions. The apply methods return the same
    /// list; this one answers before applying.
    pub fn closed_by(&self, action: &Action<P>) -> Result<Vec<P>, DockError> {
        Ok(match action {
            Action::Close { panel } => vec![panel.clone()],
            Action::CloseGroup { panel } | Action::CloseWindow { panel } => self.members(panel, unit_of(action))?,
            Action::CloseOthers { panel } => self.members(panel, Unit::Group)?.into_iter().filter(|p| p != panel).collect(),
            _ => Vec::new(),
        })
    }
}
