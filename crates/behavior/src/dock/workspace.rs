//! A saved arrangement: the current layout and where hidden panels go back. Lock Workspace and
//! Reset belong to the [`WorkspaceSet`](super::WorkspaceSet).
//!
//! Storage, file names and size caps stay with the application. A file that does not parse or
//! validate, or comes from a newer version, should be kept on disk and not overwritten.

use std::{collections::HashSet, hash::Hash};

use super::{
    DockSide,
    action::{Action, DockPlacement, Location},
    layout::Layout,
    limits::MAX_PANELS,
    policy::{DockError, Permissions},
};

/// A saved arrangement of panels: the [`Layout`] and the closed panels' former places.
///
/// Its [`revision`](Self::revision) changes when another arrangement replaces it (switch, reset, load): replace a
/// shown workspace with [`replace`](Self::replace) or the [`WorkspaceSet`](super::WorkspaceSet) operations, never by
/// assignment, so a running drag on it ends. Edits ([`apply`](Self::apply), [`set_visible`](Self::set_visible), the
/// dock's own) keep the revision.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default, bound(deserialize = "P: serde::Deserialize<'de>"))]
#[non_exhaustive]
pub struct Workspace<P> {
    /// Format version; files without one are v1 (`{layout, hidden}`). Always written as
    /// [`Workspace::VERSION`].
    #[serde(default = "first_version", serialize_with = "write_version")]
    pub version: u32,
    /// The current arrangement, changes included.
    pub layout: Layout<P>,
    /// Closed panels, and panels the application does not register now, with where each one
    /// goes back.
    pub hidden: Vec<(P, Location<P>)>,
    /// Counts the replacements of this live workspace. Not saved, and not part of equality.
    #[serde(skip)]
    revision: u64,
}

impl<P: PartialEq> PartialEq for Workspace<P> {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version && self.layout == other.layout && self.hidden == other.hidden
    }
}

fn first_version() -> u32 {
    1
}

fn write_version<S: serde::Serializer>(_: &u32, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_u32(Workspace::<()>::VERSION)
}

impl<P> Default for Workspace<P> {
    fn default() -> Self {
        Self::new(Layout::default())
    }
}

impl<P> Workspace<P> {
    /// The format version written to files.
    pub const VERSION: u32 = 3;

    /// A workspace of `layout`, with no hidden panels.
    pub fn new(layout: Layout<P>) -> Self {
        Self { version: Self::VERSION, layout, hidden: Vec::new(), revision: 0 }
    }

    /// The revision of this live workspace: it changes with every [`replace`](Self::replace) and nothing else.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Replaces the arrangement with `with` and moves to the next revision.
    pub fn replace(&mut self, with: Workspace<P>) {
        let revision = self.revision.wrapping_add(1);
        *self = Workspace { revision, ..with };
    }
}

impl<P: Clone + Eq + Hash> Workspace<P> {
    /// Check the version, the layout and that every hidden panel is hidden once and absent
    /// from the layout.
    pub fn validate(&self) -> Result<(), DockError> {
        if self.version > Self::VERSION {
            return Err(DockError::UnsupportedVersion);
        }
        self.layout.validate()?;
        if self.hidden.len() > MAX_PANELS {
            return Err(DockError::LimitExceeded);
        }
        let visible: HashSet<&P> = self.layout.panels().into_iter().collect();
        let mut seen = HashSet::new();
        for (panel, _) in &self.hidden {
            if visible.contains(panel) || !seen.insert(panel) {
                return Err(DockError::DuplicatePanel);
            }
        }
        Ok(())
    }

    /// Apply one action. Closed panels record where they go back, so each one can be reopened
    /// with [`Self::set_visible`]. `locked` is [`WorkspaceSet::locked`](super::WorkspaceSet::locked):
    /// the actions run with [`Permissions::locked`], so moving, floating and closing are
    /// rejected while activating, opening, minimizing, resizing and recording native windows
    /// still work. Returns the panels the action closed, in group order, so the application
    /// can release their state.
    pub fn apply(&mut self, action: Action<P>, locked: bool, permissions: impl Fn(&P) -> Permissions) -> Result<Vec<P>, DockError> {
        self.apply_all([action], locked, permissions)
    }

    /// Apply every action in order, or none, as [`Self::apply`] does. Returns the panels the
    /// actions closed, in order.
    pub fn apply_all(
        &mut self,
        actions: impl IntoIterator<Item = Action<P>>,
        locked: bool,
        permissions: impl Fn(&P) -> Permissions,
    ) -> Result<Vec<P>, DockError> {
        self.validate()?;
        let policy = |panel: &P| if locked { permissions(panel).locked() } else { permissions(panel) };
        let mut proposed = self.clone();
        let mut all_closed = Vec::new();
        for action in actions {
            let closed = proposed.layout.closed_by(&action)?;
            let locations = close_locations(&proposed.layout, &closed);
            // SetNative records where the window system has a floating window, so a lock
            // does not refuse it.
            if matches!(action, Action::SetNative { .. }) {
                proposed.layout.apply_with_permissions(action, &permissions)?;
            } else {
                proposed.layout.apply_with_permissions(action, policy)?;
            }
            for (panel, location) in locations {
                proposed.hidden.retain(|(p, _)| p != &panel);
                proposed.hidden.push((panel, location));
            }
            all_closed.extend(closed);
        }
        let visible: HashSet<P> = proposed.layout.panels().into_iter().cloned().collect();
        proposed.hidden.retain(|(p, _)| !visible.contains(p));
        let mut seen = HashSet::new();
        all_closed.retain(|p| !visible.contains(p) && seen.insert(p.clone()));
        proposed.validate()?;
        *self = proposed;
        Ok(all_closed)
    }

    /// Show or hide one panel. Showing restores its recorded location, following the chain of
    /// hidden anchors when its own anchor is hidden too; otherwise it opens as
    /// [`Action::Open`] with `fallback` as the anchor, following `permissions`. The hidden
    /// panels on that chain reopen beside it later, so a group or window closed one panel at a
    /// time comes back as one.
    /// A shown panel becomes the active tab of its group. Hiding follows `permissions` and
    /// `locked`, as [`Self::apply`] does, so a protected canvas stays shown.
    pub fn set_visible(
        &mut self,
        panel: P,
        visible: bool,
        fallback: Option<P>,
        locked: bool,
        permissions: impl Fn(&P) -> Permissions,
    ) -> Result<(), DockError> {
        self.validate()?;
        if !visible {
            if !self.layout.contains(&panel) {
                return Ok(());
            }
            return self.apply(Action::Close { panel }, locked, permissions).map(drop);
        }
        if self.layout.contains(&panel) {
            return self.apply(Action::Activate { panel }, false, |_| Permissions::default()).map(drop);
        }
        let mut layout = self.layout.clone();
        if self.restore_from_hidden(&mut layout, &panel).is_err() {
            layout = self.layout.clone();
            layout.apply_with_permissions(Action::Open { panel: panel.clone(), anchor: fallback }, &permissions)?;
        }
        layout.apply(Action::Activate { panel: panel.clone() })?;
        self.layout = layout;
        self.hidden.retain(|(p, _)| p != &panel);
        Ok(())
    }

    /// Restore `panel` where the chain of hidden anchors from its location ends, else at the
    /// nearest location before that end that restores. The hidden panels it passed then anchor
    /// on the panel before them in the chain, on the recorded side, so reopening them joins it
    /// instead of rebuilding their group or window a second time.
    fn restore_from_hidden(&mut self, layout: &mut Layout<P>, panel: &P) -> Result<(), DockError> {
        let find = |p: &P| self.hidden.iter().find(|(q, _)| q == p).map(|(_, location)| location);
        let mut chain = vec![(panel.clone(), find(panel).ok_or(DockError::MissingTarget)?.clone())];
        while let Some(anchor) = chain.last().and_then(|(_, location)| location.anchor.as_ref())
            && !chain.iter().any(|(p, _)| p == anchor)
            && let Some(location) = find(anchor)
        {
            chain.push((anchor.clone(), location.clone()));
        }
        // A floating location whose anchor is hidden would open a window of its own, even when
        // the chain leads to an open one. A neighbour's location brings the panel's own group
        // and column state (height, width, icon-only), not the neighbour's.
        let own = chain.first().and_then(|(_, location)| location.group.clone());
        let used = (0..chain.len())
            .rev()
            .find_map(|i| {
                let location = Location { group: own.clone().or_else(|| chain[i].1.group.clone()), ..chain[i].1.clone() };
                match layout.restore(panel.clone(), &location) {
                    Err(DockError::MissingTarget) => None,
                    result => Some(result.map(|()| i)),
                }
            })
            .unwrap_or(Err(DockError::MissingTarget))?;
        // Past an outer-edge frame the relation can't be reversed: the rest of the chain keeps
        // its forward anchors, so no two hidden panels anchor on each other.
        for i in 1..=used {
            let (before, location) = &chain[i - 1];
            let Some((_, own)) = self.hidden.iter_mut().find(|(p, _)| p == &chain[i].0) else { continue };
            let Some(location) = reversed(own, location, before) else { break };
            *own = location;
        }
        Ok(())
    }

    /// Fit a loaded workspace to the panels the application registers now. An invalid
    /// workspace is replaced by `default`. Panels not registered now (plug-ins, on-demand or
    /// numbered panels) move to `hidden` with their location, so they come back once registered.
    /// Registered panels in `default` that are neither shown nor hidden go to their default place, or to `hidden` when that place cannot be rebuilt.
    /// A registered panel that `permissions` says cannot close, such as a protected canvas, is
    /// always shown: at its recorded or default place, else at the outer edge of the docked
    /// area. Pass the panels' own permissions, not [`Permissions::locked`] ones.
    /// `hidden` keeps at most [`MAX_PANELS`] entries, dropping the oldest unregistered ones first.
    pub fn reconcile(&mut self, registered: impl Fn(&P) -> bool, permissions: impl Fn(&P) -> Permissions, default: &Workspace<P>) {
        if self.validate().is_err() {
            self.replace(default.clone());
        }
        let unknown: Vec<P> = self.layout.panels().into_iter().filter(|p| !registered(p)).cloned().collect();
        let locations = close_locations(&self.layout, &unknown);
        for panel in &unknown {
            self.layout.take(panel);
        }
        self.hidden.extend(locations);
        let known: HashSet<P> = self.layout.panels().into_iter().chain(self.hidden.iter().map(|(p, _)| p)).cloned().collect();
        let mut missing: Vec<(P, Location<P>)> = Vec::new();
        for panel in default.layout.panels() {
            if registered(panel)
                && !known.contains(panel)
                && let Ok(location) = default.layout.location(panel)
            {
                missing.push((panel.clone(), location));
            }
        }
        // Anchors may themselves be missing: place what fits, then retry the rest.
        while !missing.is_empty() {
            let before = missing.len();
            missing.retain(|(panel, location)| self.layout.restore(panel.clone(), location).is_err());
            if missing.len() == before {
                break;
            }
        }
        self.hidden.extend(missing);
        for (panel, location) in &default.hidden {
            if registered(panel) && !known.contains(panel) && !self.layout.contains(panel) && !self.hidden.iter().any(|(p, _)| p == panel) {
                self.hidden.push((panel.clone(), location.clone()));
            }
        }
        let stuck: Vec<P> = self.hidden.iter().map(|(p, _)| p).filter(|p| registered(p) && !permissions(p).closable).cloned().collect();
        for panel in stuck {
            let mut layout = self.layout.clone();
            let shown = self.restore_from_hidden(&mut layout, &panel).is_ok()
                || default.layout.location(&panel).is_ok_and(|location| layout.restore(panel.clone(), &location).is_ok())
                || layout.open_at_edge(panel.clone()).is_ok();
            if shown {
                self.layout = layout;
                self.hidden.retain(|(p, _)| p != &panel);
            }
        }
        let mut excess = self.hidden.len().saturating_sub(MAX_PANELS);
        self.hidden.retain(|(p, _)| {
            let drop = excess > 0 && !registered(p);
            excess -= usize::from(drop);
            !drop
        });
        self.hidden.truncate(MAX_PANELS);
    }
}

/// Where each of `closed` goes back, captured as if they closed last to first. Reopening them
/// first to last rebuilds the group: the first restores the group itself, the rest join it.
fn close_locations<P: Clone + Eq + Hash>(layout: &Layout<P>, closed: &[P]) -> Vec<(P, Location<P>)> {
    if let [panel] = closed {
        return layout.location(panel).map(|location| vec![(panel.clone(), location)]).unwrap_or_default();
    }
    let mut scratch = layout.clone();
    let mut locations = Vec::new();
    for panel in closed.iter().rev() {
        if let Ok(location) = scratch.location(panel) {
            locations.push((panel.clone(), location));
        }
        scratch.take(panel);
    }
    locations.reverse();
    locations
}

/// The location of a hidden panel beside `panel`, whose `location` anchors on it: the same
/// relation from the other side. `None` for an outer-edge frame, which only sat beside it.
fn reversed<P: Clone + Eq>(hidden: &Location<P>, location: &Location<P>, panel: &P) -> Option<Location<P>> {
    let next = match &hidden.placement {
        DockPlacement::Tab { before } => before.clone(),
        _ => None,
    };
    let (placement, size) = match &location.placement {
        // `before` names the anchor only when `panel` came first and the anchor second; the
        // anchor's own `before` then still names the tab after it.
        DockPlacement::Tab { before } if before == &location.anchor => (DockPlacement::Tab { before: next }, None),
        DockPlacement::Tab { .. } | DockPlacement::Split(DockSide::Center) => (DockPlacement::Tab { before: Some(panel.clone()) }, None),
        DockPlacement::Split(DockSide::Start) => (DockPlacement::Split(DockSide::End), location.size),
        DockPlacement::Split(DockSide::End) => (DockPlacement::Split(DockSide::Start), location.size),
        DockPlacement::Split(DockSide::Top) => (DockPlacement::Split(DockSide::Bottom), location.size),
        DockPlacement::Split(DockSide::Bottom) => (DockPlacement::Split(DockSide::Top), location.size),
        DockPlacement::Group { below, .. } => (DockPlacement::Group { below: !below, height: None }, None),
        DockPlacement::Column { after } => (DockPlacement::Column { after: !after }, None),
        DockPlacement::SplitSized { zone, .. } => match zone {
            DockSide::Start => (DockPlacement::Split(DockSide::End), location.size),
            DockSide::End => (DockPlacement::Split(DockSide::Start), location.size),
            DockSide::Top => (DockPlacement::Split(DockSide::Bottom), location.size),
            DockSide::Bottom => (DockPlacement::Split(DockSide::Top), location.size),
            DockSide::Center => return None,
        },
        DockPlacement::Edge(_) => return None,
    };
    Some(Location { floating: None, anchor: Some(panel.clone()), placement, size, sibling: Vec::new(), ..hidden.clone() })
}
