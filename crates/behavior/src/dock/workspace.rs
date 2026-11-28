//! A saved arrangement: the current layout and where hidden panels go back. Lock Workspace and
//! Reset belong to the [`WorkspaceSet`](super::WorkspaceSet).
//!
//! Storage, file names and size caps stay with the application. A file that does not parse or
//! validate, or comes from a newer version, should be kept on disk and not overwritten.

use std::{collections::HashSet, hash::Hash};

use craft_tracked::{Revision, Tracked};

use super::{
    DockSide,
    action::{Action, DockPlacement, Location},
    edit::tree_panels,
    groups::{Column, DockEdge, PanelPlace, Target, Unit},
    layout::Layout,
    limits::MAX_PANELS,
    node::Node,
    policy::{DockError, Permissions},
};

/// A saved arrangement of panels: the [`Layout`] and the closed panels' former places.
///
/// Two signals name its state. [`revision`](Self::revision) moves with every write: the fields are private and every
/// method that writes moves it, and a clone is a new source, so two workspace values never share a revision.
/// [`generation`](Self::generation) moves only when another arrangement replaces it (switch, reset, load): replace a
/// shown workspace with [`replace`](Self::replace) or the [`WorkspaceSet`](super::WorkspaceSet) operations, never by
/// assignment, so a running drag on it ends. Edits ([`apply`](Self::apply), [`set_visible`](Self::set_visible), the
/// dock's own) keep the generation.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(bound(deserialize = "P: serde::Deserialize<'de>"))]
#[non_exhaustive]
pub struct Workspace<P> {
    /// Format version; files without one are v1 (`{layout, hidden}`). Always written as
    /// [`Workspace::VERSION`].
    #[serde(default = "first_version", serialize_with = "write_version")]
    version: u32,
    /// The current arrangement, changes included. A save without one is no workspace: it does not load.
    layout: Layout<P>,
    /// Closed panels, and panels the application does not register now, with where each one
    /// goes back.
    #[serde(default)]
    hidden: Vec<(P, Location<P>)>,
    /// Floating panels, and closed panels that were floating, with the docked location each one left, so Dock Panel
    /// can return it; a closed panel that floats records the place it would reopen at. A closed panel's record lives
    /// as long as its `hidden` record. Saved with the workspace.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    floated: Vec<(P, Location<P>)>,
    /// Counts the replacements of this live workspace. Not saved, and not part of equality.
    #[serde(skip)]
    generation: u64,
    /// Moves with every write; a clone is a new source. Not saved, and not part of equality.
    #[serde(skip)]
    revision: Tracked<()>,
}

impl<P: PartialEq> PartialEq for Workspace<P> {
    fn eq(&self, other: &Self) -> bool {
        self.revision.revision() == other.revision.revision()
            || (self.version == other.version && self.layout == other.layout && self.hidden == other.hidden && self.floated == other.floated)
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
        Self::from_parts(layout, Vec::new(), Vec::new())
    }

    /// A workspace of `layout` with the closed panels' places `hidden` and the floating panels' docked places
    /// `floated`, unchecked, as a saved one loads; [`validate`](Self::validate) checks it.
    pub fn from_parts(layout: Layout<P>, hidden: Vec<(P, Location<P>)>, floated: Vec<(P, Location<P>)>) -> Self {
        Self { version: Self::VERSION, layout, hidden, floated, generation: 0, revision: Tracked::default() }
    }

    /// The format version the workspace was read with.
    pub fn version(&self) -> u32 {
        self.version
    }

    /// The current arrangement.
    pub fn layout(&self) -> &Layout<P> {
        &self.layout
    }

    /// The closed panels, and panels the application does not register now, with where each one goes back.
    pub fn hidden(&self) -> &[(P, Location<P>)] {
        &self.hidden
    }

    /// The floating panels, and the closed panels that were floating, with the docked location each one left.
    pub fn floated(&self) -> &[(P, Location<P>)] {
        &self.floated
    }

    /// The revision of this workspace value: it moves with every write, and no other value has it.
    pub fn revision(&self) -> Revision {
        self.revision.revision()
    }

    /// The generation of this live workspace: it moves with every [`replace`](Self::replace) and nothing else.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The workspace with its arrangement replaced by `layout`, for a stored set whose saved arrangement is kept apart.
    pub(super) fn with_layout(mut self, layout: Layout<P>) -> Self {
        self.layout = layout;
        self.touch();
        self
    }

    /// Keeps the ids of new floating windows at or above `floor`. A dock that showed windows below `floor` raises a
    /// workspace an undo brings back to it, so no id, and no OS window, goes to a second window.
    pub fn raise_window_floor(&mut self, floor: u64) {
        if self.layout.raise_window_floor(floor) {
            self.touch();
        }
    }

    /// Drops the closed-panel records of the panels `forget` names, so reopening them does not restore their place.
    pub fn forget_hidden(&mut self, forget: impl Fn(&P) -> bool) {
        let before = self.hidden.len();
        self.hidden.retain(|(panel, _)| !forget(panel));
        if self.hidden.len() != before {
            self.touch();
        }
    }

    /// Moves the revision. Every method that writes the workspace calls it.
    fn touch(&mut self) {
        self.revision.edit(|()| ());
    }

    /// Replaces the arrangement with `with` and moves to the next generation. The windows of `with` take ids this
    /// workspace never gave out, so no OS window passes to another window.
    pub fn replace(&mut self, with: Workspace<P>) {
        let generation = self.generation.wrapping_add(1);
        let revision = std::mem::take(&mut self.revision);
        let mut with = with;
        self.layout.hand_ids_to(&mut with.layout);
        *self = Workspace { generation, revision, ..with };
        self.touch();
    }
}

impl<P: Clone + Eq + Hash> Workspace<P> {
    /// Renames a panel in `layout`, `hidden` and `floated` (keys and anchors), keeping its place; all or nothing.
    /// An edit: it moves the revision and keeps the generation. `MissingPanel` when `from` is in none of them, `DuplicatePanel` when `to` is
    /// already there.
    pub fn rename(&mut self, from: &P, to: P) -> Result<(), DockError> {
        if !self.holds(from) {
            return Err(DockError::MissingPanel);
        }
        if self.holds(&to) {
            return Err(DockError::DuplicatePanel);
        }
        let swap = |panel: &mut P| {
            if *panel == *from {
                *panel = to.clone();
            }
        };
        let mut proposed = self.clone();
        if let Some(root) = proposed.layout.root.as_mut() {
            rename_in_tree(root, &swap);
        }
        for window in &mut proposed.layout.floating {
            rename_in_column(&mut window.column, &swap);
        }
        for (panel, location) in proposed.hidden.iter_mut().chain(&mut proposed.floated) {
            swap(panel);
            rename_in_location(location, &swap);
        }
        proposed.validate()?;
        let Workspace { layout, hidden, floated, .. } = proposed;
        (self.layout, self.hidden, self.floated) = (layout, hidden, floated);
        self.touch();
        Ok(())
    }

    /// Whether `panel` is in the layout or is the key of a hidden or floated record.
    fn holds(&self, panel: &P) -> bool {
        self.layout.contains(panel) || self.hidden.iter().chain(&self.floated).any(|(held, _)| held == panel)
    }

    /// Check the version, the layout, that every hidden panel is hidden once and absent
    /// from the layout and its record is bounded, and that the floated records are bounded and one per panel.
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
        for (panel, location) in &self.hidden {
            if visible.contains(panel) || !seen.insert(panel) {
                return Err(DockError::DuplicatePanel);
            }
            if location.closed_with.len() > MAX_PANELS {
                return Err(DockError::LimitExceeded);
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
        // The group of each panel as it closes, in closing order.
        let mut closing_groups = Vec::new();
        for action in actions {
            let closed = proposed.layout.closed_by(&action)?;
            closing_groups.extend(closed.iter().map(|panel| proposed.layout.members(panel, Unit::Group).unwrap_or_else(|_| vec![panel.clone()])));
            let locations = close_locations(&proposed.layout, &closed);
            let before = proposed.layout.clone();
            // Opening a hidden panel puts it back where it was.
            let reopened = match &action {
                Action::Open { panel, .. } if proposed.hidden.iter().any(|(p, _)| p == panel) => proposed.reopen(panel, &policy)?,
                _ => false,
            };
            // Floating a hidden panel records the docked place it would reopen at, without reopening it there. A panel
            // that would reopen in a floating window has no docked place.
            let hidden_place = match &action {
                Action::Float { panel, .. } if !proposed.layout.contains(panel) && proposed.hidden.iter().any(|(p, _)| p == panel) => proposed
                    .restored(panel, &policy)
                    .filter(|restored| !floats(&restored.layout, panel))
                    .and_then(|restored| restored.layout.location(panel).ok())
                    .map(|location| (panel.clone(), location)),
                _ => None,
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
            if let Some((panel, location)) = hidden_place
                && floats(&proposed.layout, &panel)
                && !proposed.floated.iter().any(|(p, _)| p == &panel)
            {
                proposed.floated.push((panel, location));
            }
            all_closed.extend(closed);
        }
        let visible: HashSet<P> = proposed.layout.panels().into_iter().cloned().collect();
        proposed.hidden.retain(|(p, _)| !visible.contains(p));
        let mut seen = HashSet::new();
        all_closed.retain(|p| !visible.contains(p) && seen.insert(p.clone()));
        proposed.record_closed_together(&all_closed, &closing_groups);
        proposed.validate()?;
        let Workspace { layout, hidden, floated, .. } = proposed;
        (self.layout, self.hidden, self.floated) = (layout, hidden, floated);
        self.touch();
        Ok(all_closed)
    }

    /// Show or hide one panel. Showing restores its recorded location, following the chain of
    /// hidden anchors when its own anchor is hidden too, and brings back the hidden panels of its
    /// group that closed with it in one batch (Close Tab Group, a preset) as tabs of its group;
    /// otherwise it opens as [`Action::Open`] with `fallback` as the anchor, following
    /// `permissions`. The other hidden panels on that chain reopen beside it later, so a group or
    /// window closed one panel at a time comes back as one.
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

    /// Restore a hidden `panel` to its recorded place, with the hidden panels closed with it, and make it the
    /// active tab. The set is restored in group order, each panel by its own record, so the group comes back in
    /// the order it had; a panel closed with it whose place refuses it stays hidden. `Ok(false)` when `panel`'s
    /// record cannot be restored, or the place it restores to refuses the panel under `permissions`; the workspace
    /// stays as it was and the caller opens the panel by the ordinary, checked path.
    fn reopen(&mut self, panel: &P, permissions: &impl Fn(&P) -> Permissions) -> Result<bool, DockError> {
        let mut restored = self.clone();
        for member in self.closed_together(panel) {
            match restored.restored(&member, permissions) {
                Some(next) => restored = next,
                None if &member == panel => return Ok(false),
                None => {}
            }
        }
        restored.layout.apply(Action::Activate { panel: panel.clone() })?;
        self.layout = restored.layout;
        self.hidden = restored.hidden;
        Ok(true)
    }

    /// The workspace with the hidden `panel` restored to its recorded place, `None` when the record cannot be
    /// restored or the place refuses the panel under `permissions`. The restore also rewrites the records of the
    /// hidden panels it passed, so it runs on a copy.
    fn restored(&self, panel: &P, permissions: &impl Fn(&P) -> Permissions) -> Option<Self> {
        let mut trial = self.clone();
        let mut layout = self.layout.clone();
        trial.restore_from_hidden(&mut layout, panel).ok()?;
        let allowed = layout
            .location(panel)
            .ok()
            .and_then(|location| location.anchor.map(|anchor| location.placement.allowed_by(permissions(&anchor))))
            .unwrap_or(true);
        if !allowed || layout.breaks_columns_beyond(&self.layout, permissions) {
            return None;
        }
        trial.layout = layout;
        trial.hidden.retain(|(p, _)| p != panel);
        Some(trial)
    }

    /// The hidden `panel` and the hidden panels closed with it, in group order: those its record's
    /// [`closed_with`](Location::closed_with) names whose own records name it back. A panel that closed again on
    /// its own since names it no more. `[panel]` when it closed alone.
    fn closed_together(&self, panel: &P) -> Vec<P> {
        let record = |p: &P| self.hidden.iter().find(|(q, _)| q == p).map(|(_, location)| location);
        let mut seen = HashSet::new();
        let together: Vec<P> = record(panel)
            .map(|location| &location.closed_with[..])
            .unwrap_or_default()
            .iter()
            .filter(|&p| p == panel || record(p).is_some_and(|location| location.closed_with.contains(panel)))
            .filter(|&p| seen.insert(p))
            .cloned()
            .collect();
        if together.len() > 1 && together.contains(panel) { together } else { vec![panel.clone()] }
    }

    /// Records on the hidden panels `closed`, which this batch closed, the panels of their group that closed with
    /// them. `groups` holds each closing panel's group as it closed, in closing order. The closed panels of one
    /// group, or of groups that share one, form one set in the order the groups name them.
    fn record_closed_together(&mut self, closed: &[P], groups: &[Vec<P>]) {
        let closed: HashSet<&P> = closed.iter().collect();
        let mut sets: Vec<Vec<P>> = Vec::new();
        for group in groups {
            let members: Vec<&P> = group.iter().filter(|p| closed.contains(p)).collect();
            let (joined, rest): (Vec<Vec<P>>, Vec<Vec<P>>) =
                std::mem::take(&mut sets).into_iter().partition(|set| set.iter().any(|p| members.contains(&p)));
            let mut set: Vec<P> = joined.into_iter().flatten().collect();
            for panel in members {
                if !set.contains(panel) {
                    set.push(panel.clone());
                }
            }
            sets = rest;
            if !set.is_empty() {
                sets.push(set);
            }
        }
        for set in sets.into_iter().filter(|set| set.len() > 1) {
            for (panel, location) in &mut self.hidden {
                if set.contains(panel) {
                    location.closed_with.clone_from(&set);
                }
            }
        }
    }

    /// Update the floated records after an action that turned `before` into the current layout: drop the stale
    /// records ([`Self::drop_stale_floated`]), and record the docked place of each panel that floats now and did not
    /// before.
    fn track_floated(&mut self, before: &Layout<P>) {
        self.drop_stale_floated();
        let layout = &self.layout;
        let fresh: Vec<(P, Location<P>)> = layout
            .panels()
            .into_iter()
            .filter(|p| floats(layout, p) && before.contains(p) && !floats(before, p) && !self.floated.iter().any(|(q, _)| q == *p))
            .filter_map(|p| before.location(p).ok().map(|location| (p.clone(), location)))
            .collect();
        self.floated.extend(fresh);
    }

    /// Drop the floated records of panels that neither float nor are hidden: a panel that closed while floating keeps
    /// its record as long as its hidden record, so it docks back to its place after it reopens floating.
    fn drop_stale_floated(&mut self) {
        let (layout, hidden) = (&self.layout, &self.hidden);
        self.floated.retain(|(p, _)| floats(layout, p) || hidden.iter().any(|(q, _)| q == p));
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
        let before = self.clone();
        self.reconcile_parts(registered, permissions, default);
        if *self != before {
            self.touch();
        }
    }

    /// [`Self::reconcile`]'s work, which leaves the revision to it.
    fn reconcile_parts(&mut self, registered: impl Fn(&P) -> bool, permissions: impl Fn(&P) -> Permissions, default: &Workspace<P>) {
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
        self.repair_columns(&permissions);
        let mut excess = self.hidden.len().saturating_sub(MAX_PANELS);
        self.hidden.retain(|(p, _)| {
            let drop = excess > 0 && !registered(p);
            excess -= usize::from(drop);
            !drop
        });
        self.hidden.truncate(MAX_PANELS);
        self.drop_stale_floated();
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

/// Applies `swap` to every panel of `root`, walking the tree with a stack: a loaded tree can be deeper than the call
/// stack.
fn rename_in_tree<P>(root: &mut Node<P>, swap: &impl Fn(&mut P)) {
    let mut visit = vec![root];
    while let Some(node) = visit.pop() {
        match node {
            Node::Split { first, second, .. } => {
                visit.push(second);
                visit.push(first);
            }
            Node::Tabs { panels, .. } => panels.iter_mut().for_each(swap),
            Node::Stack { entries } => entries.iter_mut().for_each(|entry| swap(&mut entry.panel)),
            Node::Dock { columns, .. } => columns.iter_mut().for_each(|column| rename_in_column(column, swap)),
        }
    }
}

/// Applies `swap` to every panel of the groups of `column`.
fn rename_in_column<P>(column: &mut Column<P>, swap: &impl Fn(&mut P)) {
    column.groups.iter_mut().for_each(|group| group.panels.iter_mut().for_each(swap));
}

/// Applies `swap` to the panels a location names: its anchor, its siblings and the tab it goes before.
fn rename_in_location<P>(location: &mut Location<P>, swap: &impl Fn(&mut P)) {
    location.anchor.iter_mut().for_each(swap);
    location.sibling.iter_mut().for_each(swap);
    if let DockPlacement::Tab { before: Some(panel) } = &mut location.placement {
        swap(panel);
    }
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
