//! A saved arrangement: the current layout and where hidden panels go back. Lock Workspace and
//! Reset belong to the [`WorkspaceSet`](super::WorkspaceSet).
//!
//! Storage, file names and size caps stay with the application. A file that does not parse or
//! validate, or comes from a newer version, should be kept on disk and not overwritten.

use std::{collections::HashSet, hash::Hash};

use super::{
    DockSide,
    action::{Action, DockPlacement, Location},
    edit::tree_panels,
    groups::{Column, DockEdge, PanelPlace, Target},
    layout::Layout,
    limits::MAX_PANELS,
    node::Node,
    policy::{DockError, Permissions},
};

/// A saved arrangement of panels: the [`Layout`] and the closed panels' former places.
///
/// Its [`revision`](Self::revision) changes when another arrangement replaces it (switch, reset, load): replace a
/// shown workspace with [`replace`](Self::replace) or the [`WorkspaceSet`](super::WorkspaceSet) operations, never by
/// assignment, so a running drag on it ends. Edits ([`apply`](Self::apply), [`set_visible`](Self::set_visible), the
/// dock's own) keep the revision.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(bound(deserialize = "P: serde::Deserialize<'de>"))]
#[non_exhaustive]
pub struct Workspace<P> {
    /// Format version; files without one are v1 (`{layout, hidden}`). Always written as
    /// [`Workspace::VERSION`].
    #[serde(default = "first_version", serialize_with = "write_version")]
    pub version: u32,
    /// The current arrangement, changes included. A save without one is no workspace: it does not load.
    pub layout: Layout<P>,
    /// Closed panels, and panels the application does not register now, with where each one
    /// goes back.
    #[serde(default)]
    pub hidden: Vec<(P, Location<P>)>,
    /// Floating panels with the docked location each one left, so Dock Panel can return it. Saved with the
    /// workspace.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub floated: Vec<(P, Location<P>)>,
    /// Counts the replacements of this live workspace. Not saved, and not part of equality.
    #[serde(skip)]
    revision: u64,
}

impl<P: PartialEq> PartialEq for Workspace<P> {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version && self.layout == other.layout && self.hidden == other.hidden && self.floated == other.floated
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
    /// The format version written to files. Version 4 saves a floating window as one column; older floating
    /// windows load as columns (see [`Floating`](super::Floating)).
    pub const VERSION: u32 = 4;

    /// A workspace of `layout`, with no hidden panels.
    pub fn new(layout: Layout<P>) -> Self {
        Self { version: Self::VERSION, layout, hidden: Vec::new(), floated: Vec::new(), revision: 0 }
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
    /// Check the version, the layout, that every hidden panel is hidden once and absent
    /// from the layout, and that the floated records are bounded and one per panel.
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
        if self.floated.len() > MAX_PANELS {
            return Err(DockError::LimitExceeded);
        }
        let mut recorded = HashSet::new();
        if !self.floated.iter().all(|(panel, _)| recorded.insert(panel)) {
            return Err(DockError::DuplicatePanel);
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
            let before = proposed.layout.clone();
            // Opening a hidden panel puts it back where it was.
            let reopened = match &action {
                Action::Open { panel, .. } if proposed.hidden.iter().any(|(p, _)| p == panel) => proposed.reopen(panel, &policy)?,
                _ => false,
            };
            if !reopened {
                // SetNative records where the window system has a floating window, so a lock
                // does not refuse it.
                if matches!(action, Action::SetNative { .. }) {
                    proposed.layout.apply_with_permissions(action, &permissions)?;
                } else {
                    proposed.layout.apply_with_permissions(action, policy)?;
                }
            }
            for (panel, location) in locations {
                proposed.hidden.retain(|(p, _)| p != &panel);
                proposed.hidden.push((panel, location));
            }
            proposed.track_floated(&before);
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
        let open = Action::Open { panel: panel.clone(), anchor: fallback };
        self.apply_all([open, Action::Activate { panel }], locked, permissions).map(drop)
    }

    /// Restore a hidden `panel` to its recorded place and make it the active tab. `Ok(false)` when the record
    /// cannot be restored, or the place it restores to refuses the panel under `permissions`; the workspace stays
    /// as it was and the caller opens the panel by the ordinary, checked path.
    fn reopen(&mut self, panel: &P, permissions: &impl Fn(&P) -> Permissions) -> Result<bool, DockError> {
        // The restore also rewrites the records of the hidden panels it passed, so it runs on a copy.
        let mut trial = self.clone();
        let mut layout = self.layout.clone();
        if trial.restore_from_hidden(&mut layout, panel).is_err() {
            return Ok(false);
        }
        let allowed = layout
            .location(panel)
            .ok()
            .and_then(|location| location.anchor.map(|anchor| location.placement.allowed_by(permissions(&anchor))))
            .unwrap_or(true);
        if !allowed || layout.breaks_columns_beyond(&self.layout, permissions) {
            return Ok(false);
        }
        layout.apply(Action::Activate { panel: panel.clone() })?;
        self.layout = layout;
        self.hidden = trial.hidden;
        self.hidden.retain(|(p, _)| p != panel);
        Ok(true)
    }

    /// Update the floated records after an action that turned `before` into the current layout: drop the records of
    /// panels that no longer float, and record the docked place of each panel that floats now and did not before.
    fn track_floated(&mut self, before: &Layout<P>) {
        let layout = &self.layout;
        self.floated.retain(|(p, _)| floats(layout, p));
        let fresh: Vec<(P, Location<P>)> = layout
            .panels()
            .into_iter()
            .filter(|p| floats(layout, p) && before.contains(p) && !floats(before, p) && !self.floated.iter().any(|(q, _)| q == *p))
            .filter_map(|p| before.location(p).ok().map(|location| (p.clone(), location)))
            .collect();
        self.floated.extend(fresh);
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
    /// area. A layout that breaks the column rights is repaired: a collapsed column holding a panel that does not
    /// collapse ([`Permissions::collapses`]) expands, a panel that shares no column
    /// ([`Permissions::shares_column`]) moves to a column of its own, and a panel that docks only in dock columns
    /// ([`Permissions::docks_outside_columns`]) moves from a frame to a dock column. Pass the panels' own permissions, not
    /// [`Permissions::locked`] ones.
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
        self.floated.retain(|(p, _)| registered(p));
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
        self.repair_columns(&permissions);
        let mut excess = self.hidden.len().saturating_sub(MAX_PANELS);
        self.hidden.retain(|(p, _)| {
            let drop = excess > 0 && !registered(p);
            excess -= usize::from(drop);
            !drop
        });
        self.hidden.truncate(MAX_PANELS);
    }
}

impl<P: Clone + Eq + Hash> Workspace<P> {
    /// Moves each panel that docks only in dock columns ([`Permissions::docks_outside_columns`]) out of a frame of the
    /// docked tree: to a new dock at the start edge, else a new dock at the end edge, else a column of its own at the
    /// outer side of the first docked column. A move the layout refuses leaves the panel where it was.
    fn repair_framed(&mut self, permissions: &impl Fn(&P) -> Permissions) {
        let framed: Vec<P> = self
            .layout
            .panels()
            .into_iter()
            .filter(|p| !permissions(p).docks_outside_columns && matches!(self.layout.place(p), Some(PanelPlace::Node(_))))
            .cloned()
            .collect();
        for panel in framed {
            let column_anchor =
                self.layout.panels().into_iter().find(|p| *p != &panel && matches!(self.layout.place(p), Some(PanelPlace::Dock { .. }))).cloned();
            let targets = [Some(Target::NewDock { side: DockEdge::Start }), Some(Target::NewDock { side: DockEdge::End })]
                .into_iter()
                .chain([column_anchor.map(|anchor| Target::At { anchor, placement: DockPlacement::Column { after: false } })]);
            for to in targets.flatten() {
                let mut layout = self.layout.clone();
                let moved = layout.apply_inner(Action::MovePanel { panel: panel.clone(), to }, &|p| permissions(p).is_protected()).is_ok();
                if moved && layout.validate().is_ok() {
                    self.layout = layout;
                    break;
                }
            }
        }
    }

    /// Moves each panel that shares no column but has company to a column of its own: a new dock column beside its
    /// column, a new window over its window, or a frame beside its node of the docked tree. Then expands each
    /// collapsed column that holds a panel that does not collapse, and moves each panel that docks only in dock
    /// columns out of a frame of the docked tree into a dock column ([`Self::repair_framed`]). A repair the layout
    /// refuses leaves it as it was.
    fn repair_columns(&mut self, permissions: &impl Fn(&P) -> Permissions) {
        self.repair_framed(permissions);
        let lonely: Vec<P> = self.layout.panels().into_iter().filter(|p| !permissions(p).shares_column).cloned().collect();
        for panel in lonely {
            let Some(place) = self.layout.place(&panel) else { continue };
            let company = match &place {
                PanelPlace::Node(_) => self.layout.node_of(&place).map(|node| tree_panels(node).into_iter().cloned().collect()),
                _ => self.layout.column(&place).map(|column| column.panels().cloned().collect::<Vec<P>>()),
            };
            let Some(anchor) = company.unwrap_or_default().into_iter().find(|p| p != &panel) else { continue };
            let to = match place {
                PanelPlace::Dock { .. } => Target::At { anchor, placement: DockPlacement::Column { after: false } },
                PanelPlace::Floating { window, .. } => match self.layout.floating.get(window) {
                    Some(window) => Target::Float { rect: window.rect, edge: window.edge },
                    None => continue,
                },
                PanelPlace::Node(_) => Target::At { anchor, placement: DockPlacement::Split(DockSide::Start) },
            };
            let mut layout = self.layout.clone();
            if layout.apply_inner(Action::MovePanel { panel, to }, &|p| permissions(p).is_protected()).is_ok() && layout.validate().is_ok() {
                self.layout = layout;
            }
        }
        let windows = self.layout.floating.iter_mut().map(|w| &mut w.column);
        let docked = self.layout.root.iter_mut().flat_map(dock_columns_mut);
        for column in windows.chain(docked) {
            if column.iconic && column.panels().any(|p| !permissions(p).collapses) {
                column.iconic = false;
            }
        }
    }
}

/// The columns of every dock of a docked tree.
fn dock_columns_mut<P>(root: &mut Node<P>) -> Vec<&mut Column<P>> {
    let mut out = Vec::new();
    let mut visit = vec![root];
    while let Some(node) = visit.pop() {
        match node {
            Node::Split { first, second, .. } => {
                visit.push(second);
                visit.push(first);
            }
            Node::Dock { columns, .. } => out.extend(columns.iter_mut()),
            Node::Tabs { .. } | Node::Stack { .. } => {}
        }
    }
    out
}

/// Whether `panel` is in a floating window of `layout`.
fn floats<P: Clone + Eq + Hash>(layout: &Layout<P>, panel: &P) -> bool {
    matches!(layout.place(panel), Some(PanelPlace::Floating { .. }))
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
