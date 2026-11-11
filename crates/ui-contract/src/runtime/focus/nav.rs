//! Focus movement: Tab and F6, roving groups, initial focus, restore, programmatic requests and focus after
//! removal. Every movement reads last pass's registrations and returns the egui changes to apply.

use super::{Act, BeginIn, FocusTarget, Initial, State, StopKey, StopRec};
use crate::runtime::egui::focus as egui_focus;
use craft_behavior::focus::after_removal;
use craft_behavior::nav::step;
use craft_behavior::{Arrows, MoveMods, NavMove, NavOutline, NavStep, ScopeKind};
use egui::{Context, Id};
use std::collections::HashSet;

/// One Tab stop: a component or part, or a roving group entered at its arrival member.
enum Entry {
    Stop(StopRec),
    Group(Id, StopRec),
}

impl Entry {
    fn arrival(&self) -> &StopRec {
        match self {
            Entry::Stop(rec) | Entry::Group(_, rec) => rec,
        }
    }

    fn holds(&self, current: &StopRec) -> bool {
        match self {
            Entry::Stop(rec) => rec.stop.key() == current.stop.key(),
            Entry::Group(group, _) => current.stop.group == Some(*group),
        }
    }
}

/// A roving group's members as a flat outline of enabled positions.
struct Members {
    len: usize,
    first_column: Option<usize>,
}

impl NavOutline for Members {
    fn len(&self) -> usize {
        self.len
    }
    fn first_column(&self) -> Option<usize> {
        self.first_column
    }
    fn depth(&self, _: usize) -> usize {
        0
    }
    fn has_children(&self, _: usize) -> bool {
        false
    }
    fn open(&self, _: usize) -> bool {
        false
    }
    fn is_section(&self, _: usize) -> bool {
        false
    }
    fn enabled(&self, _: usize) -> bool {
        true
    }
}

impl State {
    pub(super) fn rotate(&mut self) {
        self.last_stops = std::mem::take(&mut self.stops);
        self.last_scopes = std::mem::take(&mut self.scopes);
        self.last_scope_order = std::mem::take(&mut self.scope_order);
        self.last_groups = std::mem::take(&mut self.groups);
        self.last_row_sets = std::mem::take(&mut self.row_sets);
        self.open.clear();
        self.body_from.clear();
        self.entered = false;
    }

    /// The registered roving group `id` of last pass: a plain group, else one of a collection's row groups.
    pub(crate) fn group(&self, id: Id) -> Option<super::Group> {
        self.last_groups.get(&id).copied().or_else(|| {
            let set = self.last_row_sets.iter().find(|set| set.ids.binary_search_by_key(&id.value(), |row| row.value()).is_ok())?;
            Some(super::Group::new(id, craft_behavior::Roving::new(craft_behavior::NavShape::Flat)).row(set.owner).rtl(set.rtl))
        })
    }

    pub(super) fn find_stop(&self, key: StopKey) -> Option<StopRec> {
        self.stops.iter().chain(&self.last_stops).find(|rec| rec.stop.key() == key).cloned()
    }

    /// The stop focus is on: the recorded member or part of the focused root, else the root's own stop.
    pub(super) fn stop_of_focus(&self) -> Option<StopRec> {
        let root = self.focus_now?;
        let recorded = self.focused.get(&root).and_then(|key| self.find_stop(*key));
        recorded
            .or_else(|| self.find_stop((root, None)))
            .or_else(|| self.stops.iter().chain(&self.last_stops).find(|rec| rec.stop.root == root).cloned())
    }

    /// Moves focus to `rec`: the egui widget is its root, and the stop is recorded under that root.
    pub(super) fn focus_stop(&mut self, rec: StopRec) -> Vec<Act> {
        let stop = &rec.stop;
        let roving = stop.group.is_some();
        let keys = stop.keys;
        let escape = keys.claims.iter().any(|chord| chord.key == craft_keymap::Key::Escape);
        let filter = egui_focus::filter(
            keys.keep == super::KeepKeys::Tab,
            roving || matches!(keys.keep, super::KeepKeys::InlineArrows | super::KeepKeys::BlockArrows),
            roving || keys.keep == super::KeepKeys::BlockArrows,
            escape,
        );
        let moved = self.focus_now != Some(stop.root) || self.focused.get(&stop.root) != Some(&stop.key());
        if moved {
            self.arrived = Some((self.frame, stop.key()));
        }
        self.focused.insert(stop.root, stop.key());
        self.focus_now = Some(stop.root);
        self.revealed = Some(stop.key());
        vec![Act::Focus(stop.root, filter)]
    }

    fn in_scope(&self, scope: Option<Id>, root: Option<Id>) -> bool {
        let Some(root) = root else { return true };
        let mut at = scope;
        while let Some(id) = at {
            if id == root {
                return true;
            }
            at = self.last_scopes.get(&id).and_then(|rec| rec.parent);
        }
        false
    }

    /// The arrival member of a roving group: its selected member, else the first.
    fn arrival(&self, group: &super::Group) -> Option<StopRec> {
        let members = || self.last_stops.iter().filter(|rec| rec.stop.group == Some(group.id));
        group.selected.and_then(|key| members().find(|rec| rec.stop.key() == key)).or_else(|| members().next()).cloned()
    }

    fn entries(&self, scope_root: Option<Id>, all_controls: bool) -> Vec<Entry> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for rec in self.last_stops.iter().filter(|rec| self.in_scope(rec.scope, scope_root)) {
            match rec.stop.group.and_then(|group| self.group(group)) {
                Some(group) if group.row => {}
                Some(group) => {
                    if let Some(arrival) = seen.insert(group.id).then(|| self.arrival(&group)).flatten() {
                        out.push(Entry::Group(group.id, arrival));
                    }
                }
                None if all_controls || rec.stop.text => out.push(Entry::Stop(rec.clone())),
                None => {}
            }
        }
        out
    }

    /// The innermost trapping scope above `rec`, else the last trapping scope shown.
    fn trap_of(&self, rec: Option<&StopRec>) -> Option<Id> {
        let mut at = rec.and_then(|rec| rec.scope);
        while let Some(id) = at {
            let scope = self.last_scopes.get(&id)?;
            if scope.policy.trap {
                return Some(id);
            }
            at = scope.parent;
        }
        if rec.is_some() {
            return None;
        }
        self.last_scope_order.iter().rev().find(|id| self.last_scopes.get(id).is_some_and(|scope| scope.policy.trap)).copied()
    }

    pub(super) fn tab(&mut self, forward: bool, all_controls: bool) -> Vec<Act> {
        // A child of a collection row tabs on from its collection: the row groups are no Tab entries.
        let current = self.stop_of_focus().map(|rec| {
            let owner = rec.stop.group.and_then(|group| self.group(group)).filter(|group| group.row).and_then(|group| self.row_owner(&group));
            owner.unwrap_or(rec)
        });
        let entries = self.entries(self.trap_of(current.as_ref()), all_controls);
        let Some(next) = cycle(&entries, current.as_ref(), forward) else { return Vec::new() };
        let arrival = entries[next].arrival().clone();
        self.focus_stop(arrival)
    }

    /// F6: the first stop of the next pane, with its label as the announcement.
    pub(super) fn pane(&mut self, forward: bool, all_controls: bool) -> (Vec<Act>, Vec<String>) {
        let panes: Vec<Id> = self
            .last_scope_order
            .iter()
            .copied()
            .filter(|id| self.last_scopes.get(id).is_some_and(|scope| scope.policy.kind == ScopeKind::Pane))
            .collect();
        let current = self.stop_of_focus();
        let here = current.as_ref().and_then(|rec| self.pane_of(rec));
        let at = here.and_then(|pane| panes.iter().position(|id| *id == pane));
        let next = match (at, forward) {
            (Some(at), true) => (at + 1) % panes.len(),
            (Some(at), false) => (at + panes.len() - 1) % panes.len(),
            (None, true) => 0,
            (None, false) => panes.len().saturating_sub(1),
        };
        let Some(pane) = panes.get(next).copied() else { return (Vec::new(), Vec::new()) };
        let Some(entry) = self.entries(Some(pane), all_controls).into_iter().next() else { return (Vec::new(), Vec::new()) };
        let label = self.last_scopes.get(&pane).and_then(|scope| scope.label.as_ref()).map(|label| label.to_string());
        (self.focus_stop(entry.arrival().clone()), label.into_iter().collect())
    }

    pub(super) fn pane_of(&self, rec: &StopRec) -> Option<Id> {
        let mut at = rec.scope;
        while let Some(id) = at {
            let scope = self.last_scopes.get(&id)?;
            if scope.policy.kind == ScopeKind::Pane {
                return Some(id);
            }
            at = scope.parent;
        }
        None
    }

    pub(super) fn apply_target(&mut self, target: FocusTarget) -> Vec<Act> {
        let key = match target {
            FocusTarget::Id(id) => (id, None),
            FocusTarget::Part(id, part) => (id, Some(part)),
            FocusTarget::Doc(doc) => match self.docs.get(&doc) {
                Some(id) => (*id, None),
                None => return Vec::new(),
            },
        };
        let found = self.find_stop(key).or_else(|| self.stops.iter().chain(&self.last_stops).find(|rec| rec.stop.id == key.0).cloned());
        match found {
            Some(rec) => self.focus_stop(rec),
            None => {
                self.revealed = Some(key);
                self.focus_now = Some(key.0);
                vec![Act::Focus(key.0, Default::default())]
            }
        }
    }

    /// A roving move of the group of `from`: focus moves to the member the navigation core picks. A one-line text
    /// stop in the body of a group that moves with Up and Down only, which it did not join, hands the group those
    /// keys: they enter the group at its first or last member.
    fn roving_move(&mut self, from: StopKey, mv: NavMove, mods: MoveMods, horizontal: bool) -> Option<Vec<Act>> {
        let current = self.find_stop(from)?;
        let (group, member) = match current.stop.group {
            Some(group) => (self.group(group)?, true),
            None => (self.group(current.stop.body?)?, false),
        };
        let hands_on = || {
            current.stop.text
                && !current.stop.role.value
                && matches!(group.roving.arrows, Arrows::Vertical)
                && matches!(mv, NavMove::Up | NavMove::Down)
        };
        if !member && !hands_on() {
            return None;
        }
        let mv = match (horizontal && group.rtl, mv) {
            (true, NavMove::Back) => NavMove::Forward,
            (true, NavMove::Forward) => NavMove::Back,
            (_, mv) => mv,
        };
        if !group.roving.takes(mv) {
            return None;
        }
        let members: Vec<StopRec> = self.last_stops.iter().filter(|rec| rec.stop.group == Some(group.id)).cloned().collect();
        let lead = group.lead.unwrap_or(from);
        let at = members.iter().position(|rec| rec.stop.key() == lead);
        if group.row && mv == NavMove::Back && at == Some(0) {
            return Some(self.row_owner(&group).map(|rec| self.focus_stop(rec)).unwrap_or_default());
        }
        let mut state = group.roving.state(at);
        match step(group.roving.shape, &Members { len: members.len(), first_column: group.roving.first_column }, &mut state, mv, mods) {
            NavStep::Moved(to) => Some(self.focus_stop(members[to].clone())),
            NavStep::Stay if group.roving.hands_back => None,
            _ => Some(Vec::new()),
        }
    }

    /// The stop of the collection a row group belongs to.
    fn row_owner(&self, group: &super::Group) -> Option<StopRec> {
        self.find_stop((group.owner?, None))
    }

    /// Moves focus from a child of a collection row back to the collection; `None` when `from` is no row child.
    pub(super) fn leave_row(&mut self, from: StopKey) -> Option<(Id, Vec<Act>)> {
        let group = self.group(self.find_stop(from)?.stop.group?)?;
        if !group.row {
            return None;
        }
        let owner = self.row_owner(&group)?;
        let id = owner.stop.id;
        Some((id, self.focus_stop(owner)))
    }

    /// Moves focus to the arrival member of the row group `group`.
    pub(super) fn enter_row(&mut self, group: Id) -> Option<Vec<Act>> {
        let group = self.group(group)?;
        let member = self.arrival(&group)?;
        Some(self.focus_stop(member))
    }

    /// The stop that takes focus when the focused stop was not registered this pass.
    fn removed_focus(&self, focused: Option<Id>) -> Option<StopRec> {
        let root = focused?;
        let key = *self.focused.get(&root)?;
        if self.stops.iter().any(|rec| rec.stop.key() == key) {
            return None;
        }
        let gone = self.last_stops.iter().find(|rec| rec.stop.key() == key)?;
        // A removed child of a collection row hands focus back to the row, as Escape does, not to the next stop.
        let row = gone.stop.group.and_then(|group| self.group(group)).filter(|group| group.row);
        if let Some(owner) =
            row.and_then(|group| self.row_owner(&group)).filter(|owner| self.stops.iter().any(|rec| rec.stop.key() == owner.stop.key()))
        {
            return Some(owner);
        }
        let order: Vec<StopKey> = self.last_stops.iter().filter(|rec| rec.scope == gone.scope).map(|rec| rec.stop.key()).collect();
        let registered = |candidate: &StopKey| self.stops.iter().any(|rec| rec.stop.key() == *candidate);
        match after_removal(&order, &key, registered) {
            Some(next) => self.stops.iter().find(|rec| rec.stop.key() == *next).cloned(),
            None => self.stops.iter().find(|rec| rec.scope == gone.scope).cloned(),
        }
    }

    /// The opener of a scope that went away while focus was inside it.
    fn restored_focus(&self, focused: Option<Id>) -> Option<StopRec> {
        let root = focused?;
        let key = *self.focused.get(&root).unwrap_or(&(root, None));
        let inside = self.last_stops.iter().find(|rec| rec.stop.key() == key)?;
        let opener = self.opener_back(inside.scope, |id| self.scopes.get(&id).is_none_or(|now| now.closing))?;
        self.stops.iter().find(|rec| rec.stop.key() == opener).cloned()
    }

    /// Where a menu pick returns focus before its command runs: the pick closes the chain of menus around the focused
    /// stop, so focus goes to the opener of the outermost one, or to the stop a menu bar took focus from when that
    /// opener is the bar. `None` when focus is in no menu.
    pub(super) fn picked_focus(&mut self) -> Option<StopRec> {
        let inside = self.stop_of_focus()?;
        let menu = |id: Id| self.scopes.get(&id).or_else(|| self.last_scopes.get(&id)).is_some_and(|scope| scope.policy.kind == ScopeKind::Menu);
        let mut opener = self.opener_back(inside.scope, menu)?;
        if let Some((bar, held)) = self.bar_return
            && opener.0 == bar
        {
            self.bar_return = None;
            opener = held;
        }
        self.find_stop(opener)
    }

    /// Walking out from `from`, the opener focus returns to when the scopes `closes` names go away: the opener of the
    /// outermost restoring scope in the first run of closing scopes, so a closed chain of submenus returns focus to
    /// where its outermost menu was opened from.
    fn opener_back(&self, from: Option<Id>, closes: impl Fn(Id) -> bool) -> Option<StopKey> {
        let mut at = from;
        let mut back = None;
        while let Some(id) = at {
            let scope = self.scopes.get(&id).or_else(|| self.last_scopes.get(&id))?;
            if closes(id) {
                if scope.policy.restore {
                    back = scope.opener;
                }
            } else if back.is_some() {
                break;
            }
            at = scope.parent;
        }
        back
    }
}

fn cycle(entries: &[Entry], current: Option<&StopRec>, forward: bool) -> Option<usize> {
    if entries.is_empty() {
        return None;
    }
    let at = current.and_then(|rec| entries.iter().position(|entry| entry.holds(rec)));
    Some(match (at, forward) {
        (Some(at), true) => (at + 1) % entries.len(),
        (Some(at), false) => (at + entries.len() - 1) % entries.len(),
        (None, true) => 0,
        (None, false) => entries.len() - 1,
    })
}

/// Puts the stops of the innermost open scope in Tab order: the body's, then the footer's, then the header's.
/// Inner scopes closed already and lie within one region, so they move with it.
pub(super) fn order_regions(state: &mut State) {
    let Some((scope, first)) = state.open.last().copied() else { return };
    let Some(body) = state.body_from.remove(&scope) else { return };
    let end = state.stops.len();
    let first = first.min(end);
    state.stops[first..end].rotate_left(body.clamp(first, end) - first);
}

/// Closes the innermost open scope; a scope shown for the first time moves focus to its initial stop.
pub(super) fn initial_focus(state: &mut State, frame: u64) -> Vec<Act> {
    let Some((scope, first)) = state.open.pop() else { return Vec::new() };
    let Some((policy, entering)) = state.scopes.get(&scope).map(|rec| (rec.policy, rec.entering)) else { return Vec::new() };
    let new = match entering {
        Some(appears) => appears,
        None => {
            state.seen.retain(|_, last| *last + 1 >= frame);
            let previous = state.seen.insert(scope, frame);
            !previous.is_some_and(|last| last + 1 >= frame)
        }
    };
    if !new {
        return Vec::new();
    }
    let stops = &state.stops[first.min(state.stops.len())..];
    let pick = match policy.initial {
        Initial::First => stops.first(),
        Initial::Default => stops.iter().find(|rec| rec.stop.default).or_else(|| stops.first()),
        Initial::Key(name) => stops.iter().find(|rec| rec.stop.name == Some(name)),
        _ => None,
    };
    match pick.cloned() {
        Some(rec) => {
            state.entered = true;
            state.focus_stop(rec)
        }
        None => Vec::new(),
    }
}

/// The begin hook's focus moves: a pending request, Tab and F6, then the press classification.
pub(super) fn begin(state: &mut State, input: BeginIn) -> (Vec<Act>, Vec<String>) {
    state.rotate();
    state.frame = input.frame;
    state.focus_now = input.focused;
    state.press.rotate(input.focused);
    let mut acts = Vec::new();
    let mut announcements = Vec::new();
    if let Some(target) = state.pending.take() {
        acts.extend(state.apply_target(target));
    }
    for forward in input.nav.tabs {
        let moved = state.tab(forward, input.tab_all_controls);
        if let Some(from) = input.focused.filter(|_| !moved.is_empty()) {
            state.press.tabbed_from = Some((from, forward));
        }
        acts.extend(moved);
    }
    for forward in input.nav.panes {
        let (moved, said) = state.pane(forward, input.tab_all_controls);
        acts.extend(moved);
        announcements.extend(said);
    }
    if let Some(at) = &input.press {
        state.classify_press(at);
    }
    (acts, announcements)
}

/// The end hook's focus moves, and the record the next input hook reads.
pub(super) fn end(state: &mut State, focused: Option<Id>, frame: u64) -> Vec<Act> {
    // The moves below belong to the next frame's passes.
    state.frame = frame + 1;
    let mut acts = state.apply_press(focused);
    // A layer that appeared in this pass took focus already: the press that opened it does not take it back.
    if state.entered {
        acts.clear();
    }
    if acts.is_empty() {
        let next = state.restored_focus(focused).or_else(|| state.removed_focus(focused));
        if let Some(rec) = next {
            acts = state.focus_stop(rec);
        }
    }
    let now = acts.last().map_or(focused, |act| match act {
        Act::Focus(id, _) => Some(*id),
        Act::Surrender(_) => None,
    });
    state.focus_now = now;
    state.focused.retain(|root, _| Some(*root) == now);
    let stop = state.stop_of_focus().map(|rec| rec.stop);
    let keys = stop.as_ref().map(|stop| stop.keys);
    state.record = super::FocusedRecord {
        claims: keys.map(|keys| keys.claims.to_vec()).unwrap_or_default(),
        focused: stop.as_ref().map(|stop| stop.id),
        pages: stop.as_ref().is_some_and(|stop| stop.role.pages),
        keys,
        menu_bar: std::mem::take(&mut state.menu_bar),
    };
    acts
}

/// Applies a roving move of the group that `from` belongs to. `None` when `from` is no group member or the
/// group does not take the move; the caller then routes the move to the component.
pub(crate) fn roving_move(ctx: &Context, from: StopKey, mv: NavMove, mods: MoveMods, horizontal: bool) -> Option<()> {
    let acts = super::with(ctx, |state| state.roving_move(from, mv, mods, horizontal))?;
    super::apply(ctx, acts);
    Some(())
}

/// Moves focus from a child of a collection row back to the row's collection. Returns the collection's id, or
/// `None` when `from` is no row child.
pub(crate) fn leave_row(ctx: &Context, from: StopKey) -> Option<Id> {
    let (owner, acts) = super::with(ctx, |state| state.leave_row(from))?;
    super::apply(ctx, acts);
    Some(owner)
}

/// Moves focus into the first child or part of the row group `group`; `false` when it has none.
pub(crate) fn enter_row(ctx: &Context, group: Id) -> bool {
    let Some(acts) = super::with(ctx, |state| state.enter_row(group)) else { return false };
    super::apply(ctx, acts);
    true
}

/// Whether the row group `group` has children or parts that are focus stops, as of last pass.
pub(crate) fn row_has_stops(ctx: &Context, group: Id) -> bool {
    super::with(ctx, |state| state.last_stops.iter().any(|rec| rec.stop.group == Some(group)))
}
