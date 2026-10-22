use std::{collections::HashSet, hash::Hash};

use egui::Rect;

use crate::layout::{SplitAxis, SplitSize};

use super::{DockError, DockStyle, Layout, Node, PanelLimits, Permissions, measure_layout_with_boundaries};

/// Stable neighbors identify the divider after removal of the moving source members.
pub struct BetweenRequest<'a, P> {
    pub members: &'a [P],
    pub selected: &'a P,
    pub before: &'a P,
    pub after: &'a P,
}

/// Insert a tab/group between split children atomically. New space starts at one third;
/// remaining space preserves the neighbors' measured proportion, subject to pane limits.
pub fn insert_between<P: Clone + Eq + Hash>(
    layout: &mut Layout<P>,
    request: BetweenRequest<'_, P>,
    workspace: Rect,
    screen: Rect,
    style: &DockStyle,
    permissions: impl Fn(&P) -> Permissions,
    limits: impl Fn(&P) -> PanelLimits,
) -> Result<(), DockError> {
    layout.validate()?;
    if request.members.is_empty()
        || request.members.len() > super::MAX_PANELS
        || request.members.iter().collect::<HashSet<_>>().len() != request.members.len()
        || !request.members.contains(request.selected)
        || request.before == request.after
        || request.members.contains(request.before)
        || request.members.contains(request.after)
    {
        return Err(DockError::InvalidPlacement);
    }
    if request.members.iter().any(|panel| !permissions(panel).movable)
        || !permissions(request.before).accepts_tabs
        || !permissions(request.after).accepts_tabs
    {
        return Err(DockError::NotAllowed);
    }
    measure_layout_with_boundaries(layout, workspace, screen, style, &permissions, &limits)?;
    let mut proposed = layout.clone();
    for panel in request.members {
        proposed.take_for_transfer(panel)?;
    }
    let measured = measure_layout_with_boundaries(&proposed, workspace, screen, style, &permissions, &limits)?;
    let boundary = measured
        .boundaries
        .iter()
        .find(|boundary| boundary.first.contains(request.before) && boundary.second.contains(request.after))
        .ok_or(DockError::MissingTarget)?;
    let active = request.members.iter().position(|panel| panel == request.selected).ok_or(DockError::InvalidSelection)?;
    let incoming = Node::Tabs { panels: request.members.to_vec(), active };
    let root = proposed.root.as_mut().ok_or(DockError::MissingTarget)?;
    insert_with_policy(
        root,
        request.before,
        request.after,
        &Policy { incoming: &incoming, boundary, style, permissions: &permissions, limits: &limits },
    )?;
    proposed.validate()?;
    *layout = proposed;
    Ok(())
}

fn contains<P: Eq>(node: &Node<P>, panel: &P) -> bool {
    match node {
        Node::Tabs { panels, .. } => panels.contains(panel),
        Node::Stack { entries } => entries.iter().any(|entry| &entry.panel == panel),
        Node::Split { first, second, .. } => contains(first, panel) || contains(second, panel),
    }
}

struct Policy<'a, P, F, L> {
    incoming: &'a Node<P>,
    boundary: &'a super::SplitBoundary<P>,
    style: &'a DockStyle,
    permissions: &'a F,
    limits: &'a L,
}

fn insert_with_policy<P: Clone + Eq, F: Fn(&P) -> Permissions, L: Fn(&P) -> PanelLimits>(
    node: &mut Node<P>,
    before: &P,
    after: &P,
    policy: &Policy<'_, P, F, L>,
) -> Result<(), DockError> {
    let Node::Split { axis, first, second, .. } = node else { return Err(DockError::MissingTarget) };
    if contains(first, before) && contains(second, after) {
        if *axis != policy.boundary.axis {
            return Err(DockError::InvalidPlacement);
        }
        let a = super::surface::node_limits(first, policy.style, policy.permissions, policy.limits);
        let b = super::surface::node_limits(second, policy.style, policy.permissions, policy.limits);
        let n = super::surface::node_limits(policy.incoming, policy.style, policy.permissions, policy.limits);
        let component = |size: egui::Vec2| if *axis == SplitAxis::Horizontal { size.x } else { size.y };
        let cross = |size: egui::Vec2| if *axis == SplitAxis::Horizontal { size.y } else { size.x };
        let available = component(policy.boundary.rect.size()) - 2.0 * policy.style.gap;
        let low = component(n.min).max(available - component(a.max) - component(b.max));
        let high = component(n.max).min(available - component(a.min) - component(b.min));
        if available <= 0.0 || low > high || cross(policy.boundary.rect.size()) < cross(a.min).max(cross(b.min)).max(cross(n.min)) {
            return Err(DockError::InvalidGeometry);
        }
        let new_extent = (available / 3.0).clamp(low, high);
        let remaining = available - new_extent;
        let low = component(a.min).max(remaining - component(b.max));
        let high = component(a.max).min(remaining - component(b.min));
        if low > high {
            return Err(DockError::InvalidGeometry);
        }
        let old_a = component(policy.boundary.first_rect.size());
        let old_b = component(policy.boundary.second_rect.size());
        if old_a + old_b <= 0.0 {
            return Err(DockError::InvalidGeometry);
        }
        let a_extent = (remaining * old_a / (old_a + old_b)).clamp(low, high);
        let replacement = Node::Split {
            axis: *axis,
            size: SplitSize::FixedFirst(a_extent),
            first: first.clone(),
            second: Box::new(Node::Split {
                axis: *axis,
                size: SplitSize::FixedFirst(new_extent),
                first: Box::new(policy.incoming.clone()),
                second: second.clone(),
            }),
        };
        *node = replacement;
        Ok(())
    } else if contains(first, before) && contains(first, after) {
        insert_with_policy(first, before, after, policy)
    } else if contains(second, before) && contains(second, after) {
        insert_with_policy(second, before, after, policy)
    } else {
        Err(DockError::InvalidPlacement)
    }
}
