//! Focus movement: Tab and F6, roving groups, initial focus, restore, programmatic requests and focus after
//! removal. Every movement reads last pass's registrations and returns the egui changes to apply.

use super::{Act, BeginIn, FocusOrigin, FocusTarget, Initial, PickedIn, State, Stop, StopKey, StopRec, TabReach};
use crate::runtime::egui::focus as egui_focus;
use crate::runtime::egui::memory::FastMap;
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

    /// Whether the entry holds `current`, whose outermost roving group is `top`.
    fn holds(&self, current: &StopRec, top: Option<Id>) -> bool {
        match self {
            Entry::Stop(rec) => rec.stop.key() == current.stop.key(),
            Entry::Group(group, _) => top == Some(*group),
        }
    }
}

/// The most parent links a roving group's chain is followed through. A part group joins a body group, which joins
/// nothing, so one link is all a real chain has; the bound ends a malformed one.
const MAX_NESTING: usize = 8;

/// One member of a roving group: a stop, or a nested group that the group steps over as a whole.
#[derive(Clone)]
enum Slot {
    Stop(StopRec),
    Group(Id),
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

/// The walk out of a layer (`State::walk_out`).
pub(super) struct MenuWalk {
    /// The stop focus goes to once every crossed scope closes; `None` when a crossed scope returns focus nowhere or
    /// the chain loops.
    pub(super) to: Option<StopRec>,
    /// The layer the walk started from, then each menu it crossed, innermost first.
    pub(super) crossed: Vec<Id>,
}

/// One pass's registrations: the stops, roving groups and row groups Tab order is made of.
#[derive(Clone, Copy)]
struct Lists<'a> {
    stops: &'a [StopRec],
    groups: &'a FastMap<Id, super::Group>,
    row_sets: &'a [super::RowGroups],
}

impl Lists<'_> {
    /// The roving group `id`: a plain group, else one of a collection's row groups.
    fn group(&self, id: Id) -> Option<super::Group> {
        self.groups.get(&id).copied().or_else(|| {
            let set = self.row_sets.iter().find(|set| set.ids.binary_search_by_key(&id.value(), |row| row.value()).is_ok())?;
            Some(super::Group::new(id, craft_behavior::Roving::new(craft_behavior::NavShape::Flat)).row(set.owner).rtl(set.rtl))
        })
    }

    /// The member of `group` that `rec` is or sits in: `rec` itself, or the nested group of `group` whose chain holds
    /// it. `None` when `rec` is in neither.
    fn slot_in(&self, rec: &StopRec, group: Id) -> Option<Slot> {
        let mut at = rec.stop.group?;
        if at == group {
            return Some(Slot::Stop(rec.clone()));
        }
        for _ in 0..MAX_NESTING {
            let parent = self.group(at)?.parent?;
            if parent == group {
                return Some(Slot::Group(at));
            }
            at = parent;
        }
        None
    }

    /// The members of `group` in reading order: its own stops, and each nested group once, where its first stop sits.
    fn slots(&self, group: Id) -> Vec<Slot> {
        let mut seen = HashSet::new();
        self.stops
            .iter()
            .filter_map(|rec| self.slot_in(rec, group))
            .filter(|slot| match slot {
                Slot::Group(id) => seen.insert(*id),
                Slot::Stop(_) => true,
            })
            .collect()
    }

    /// The outermost group that `group` is nested in; `group` itself when it has no parent.
    fn outermost(&self, group: Id) -> Id {
        let mut at = group;
        for _ in 0..MAX_NESTING {
            match self.group(at).and_then(|group| group.parent) {
                Some(parent) => at = parent,
                None => break,
            }
        }
        at
    }

    /// The arrival member of a roving group: its selected member, else the first, entering a nested group at its own
    /// arrival.
    fn arrival(&self, group: &super::Group) -> Option<StopRec> {
        let mut group = *group;
        for _ in 0..MAX_NESTING {
            let selected =
                group.selected.and_then(|key| self.stops.iter().find(|rec| rec.stop.key() == key && self.slot_in(rec, group.id).is_some()));
            if let Some(rec) = selected {
                return Some(rec.clone());
            }
            match self.slots(group.id).into_iter().next()? {
                Slot::Stop(rec) => return Some(rec),
                Slot::Group(id) => group = self.group(id)?,
            }
        }
        None
    }

    /// The Tab entries among the stops `keep` takes: a roving group once, at its arrival member, a nested group within
    /// its outermost group's entry; a collection's row groups none; another stop when Tab moves over all controls or
    /// it holds text, unless only a press reaches it.
    fn entries(&self, keep: impl Fn(&StopRec) -> bool, all_controls: bool) -> Vec<Entry> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for rec in self.stops.iter().filter(|rec| keep(rec)) {
            match rec.stop.group.and_then(|group| self.group(self.outermost(group))) {
                Some(group) if group.row => {}
                Some(group) => {
                    if let Some(arrival) = seen.insert(group.id).then(|| self.arrival(&group)).flatten() {
                        out.push(Entry::Group(group.id, arrival));
                    }
                }
                None if !rec.stop.press_only && (all_controls || rec.stop.text) => out.push(Entry::Stop(rec.clone())),
                None => {}
            }
        }
        out
    }
}

impl State {
    pub(super) fn rotate(&mut self) {
        self.last_stops = std::mem::take(&mut self.stops);
        self.stop_ids.clear();
        self.stop_roots.clear();
        self.last_scopes = std::mem::take(&mut self.scopes);
        self.last_scope_order = std::mem::take(&mut self.scope_order);
        self.last_groups = std::mem::take(&mut self.groups);
        self.last_row_sets = std::mem::take(&mut self.row_sets);
        self.last_docs = std::mem::take(&mut self.docs);
        self.open.clear();
        self.body_from.clear();
        self.body_end.clear();
        self.entered = false;
        self.asked = false;
        self.candidates_seen = 0;
        self.host_canvases.clear();
    }

    /// The registrations of last pass, which Tab and the arrows move over.
    fn last(&self) -> Lists<'_> {
        Lists { stops: &self.last_stops, groups: &self.last_groups, row_sets: &self.last_row_sets }
    }

    /// The registrations of the running pass so far.
    fn this(&self) -> Lists<'_> {
        Lists { stops: &self.stops, groups: &self.groups, row_sets: &self.row_sets }
    }

    /// The registered roving group `id` of last pass: a plain group, else one of a collection's row groups.
    pub(crate) fn group(&self, id: Id) -> Option<super::Group> {
        self.last().group(id)
    }

    /// The roving group `id` as registered in the running pass so far, else in the last.
    pub(crate) fn current_group(&self, id: Id) -> Option<super::Group> {
        self.this().group(id).or_else(|| self.last().group(id))
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

    /// Whether `id`, holding egui focus, is an app's own widget: as `made` says, no widget a component made
    /// (`pointer::made_by_component`), and the root of no stop but the one every focusable egui widget of the app has
    /// (`Stop::automatic`). The app's canvas (`host_canvas`, `KeyUse::chrome_key`) is none: its declared
    /// keys route as a framework stop's do, so egui's arrows and Tab never move focus off it.
    pub(super) fn app_widget(&self, id: Id, made: bool) -> bool {
        !made && !self.stops.iter().chain(&self.last_stops).any(|rec| !rec.stop.automatic() && (rec.stop.root == id || rec.stop.key() == (id, None)))
    }

    /// Moves focus to `rec` and asks the enclosing scroll owner to reveal it: the egui widget is its root, and the stop
    /// is recorded under that root.
    pub(super) fn focus_stop(&mut self, rec: StopRec) -> Vec<Act> {
        self.focus_stop_from(rec, FocusOrigin::Request)
    }

    /// [`focus_stop`](Self::focus_stop) with what moved focus there. A row that takes the focus back from its removed
    /// child is not revealed: nothing chose to show it.
    fn focus_stop_from(&mut self, rec: StopRec, origin: FocusOrigin) -> Vec<Act> {
        let key = rec.stop.key();
        let acts = self.enter_stop(rec, origin);
        if origin != FocusOrigin::Removal {
            self.revealed = Some(key);
        }
        acts
    }

    /// Moves focus to `rec` without a reveal: a press focuses a stop under the pointer, which is on screen already,
    /// and scrolling it in would move the content under the held press.
    pub(super) fn focus_stop_in_place(&mut self, rec: StopRec) -> Vec<Act> {
        self.enter_stop(rec, FocusOrigin::Press)
    }

    fn enter_stop(&mut self, rec: StopRec, origin: FocusOrigin) -> Vec<Act> {
        self.note_entry(&rec);
        let stop = &rec.stop;
        let moved = self.focus_now != Some(stop.root) || self.focused.get(&stop.root) != Some(&stop.key());
        if moved {
            self.arrived = Some((self.frame, stop.key(), origin));
        }
        self.focused.insert(stop.root, stop.key());
        self.focus_now = Some(stop.root);
        vec![focus_act(stop)]
    }

    /// Focus moving to `rec` from outside a scope that traps nothing and restores focus (a layer that Tab reaches, such as
    /// the toasts) makes the stop it comes from that scope's opener: when the scope goes away with focus inside, focus
    /// returns there.
    fn note_entry(&mut self, rec: &StopRec) {
        let Some(from) = self.stop_of_focus() else { return };
        let mut at = rec.scope;
        while let Some(id) = at {
            let Some(scope) = self.scopes.get(&id).or_else(|| self.last_scopes.get(&id)) else { return };
            let (policy, parent) = (scope.policy, scope.parent);
            if policy.restore && !policy.trap && !self.in_scope(from.scope, Some(id)) {
                for scopes in [&mut self.scopes, &mut self.last_scopes] {
                    if let Some(scope) = scopes.get_mut(&id) {
                        scope.opener = Some(from.stop.key());
                    }
                }
            }
            at = parent;
        }
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

    /// The stop that opened the innermost scope around `scope` that keeps the focus on its anchor (`Initial::KeepAnchor`,
    /// a dropdown's layer); `None` outside every such scope.
    fn anchored_layer(&self, scope: Option<Id>) -> Option<StopKey> {
        let mut at = scope;
        while let Some(id) = at {
            let rec = self.last_scopes.get(&id)?;
            if rec.policy.initial == Initial::KeepAnchor {
                return rec.opener;
            }
            at = rec.parent;
        }
        None
    }

    /// The arrival member of a roving group of last pass: its selected member, else the first.
    fn arrival(&self, group: &super::Group) -> Option<StopRec> {
        self.last().arrival(group)
    }

    fn entries(&self, scope_root: Option<Id>, all_controls: bool) -> Vec<Entry> {
        self.last().entries(|rec| self.reaches(rec.scope, scope_root), all_controls)
    }

    /// Whether Tab and F6 inside the scope `root` reach a stop of `scope`: it lies inside `root`, and not inside a
    /// layer opened within `root` that takes no focus, traps nothing and restores nothing (a menu's companion bar).
    fn reaches(&self, scope: Option<Id>, root: Option<Id>) -> bool {
        let Some(root) = root else { return true };
        let mut at = scope;
        while let Some(id) = at {
            if id == root {
                return true;
            }
            let Some(rec) = self.last_scopes.get(&id) else { return false };
            let unreached = rec.entering.is_some() && !rec.policy.trap && !rec.policy.restore && rec.policy.initial == Initial::Keep;
            if unreached {
                return false;
            }
            at = rec.parent;
        }
        false
    }

    /// The stops of the running pass that keyboard focus reaches, among those `keep` takes, in Tab order: every member
    /// of a roving group, in member order, where Tab enters the group; no row of a collection (Tab stops on the
    /// collection); every other stop, whatever the platform's Tab setting.
    pub(super) fn tab_order(&self, keep: impl Fn(&StopRec) -> bool) -> Vec<StopKey> {
        let lists = self.this();
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for rec in self.stops.iter().filter(|rec| keep(rec)) {
            match rec.stop.group.and_then(|group| lists.group(lists.outermost(group))) {
                Some(group) if group.row => {}
                Some(group) => {
                    if seen.insert(group.id) {
                        let members = self
                            .stops
                            .iter()
                            .filter(|member| member.stop.group.is_some_and(|inner| lists.outermost(inner) == group.id) && keep(member));
                        out.extend(members.map(|member| member.stop.key()));
                    }
                }
                None => out.push(rec.stop.key()),
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

    /// A Tab move. In a dialog's trap with `dialog_tab` at `Fields`, only text stops are entries, whatever the
    /// window's rule: a roving group whose arrival member holds no text is skipped too.
    pub(super) fn tab(&mut self, forward: bool, all_controls: bool, dialog_tab: TabReach) -> Vec<Act> {
        // A child of a collection row tabs on from its collection: the row groups are no Tab entries. The content of a
        // layer that keeps the focus on its anchor (a dropdown) is no Tab entry either: Tab goes on from the anchor.
        let current = self.stop_of_focus().map(|rec| {
            let owner = rec.stop.group.and_then(|group| self.group(group)).filter(|group| group.row).and_then(|group| self.row_owner(&group));
            let rec = owner.unwrap_or(rec);
            self.anchored_layer(rec.scope).and_then(|opener| self.find_stop(opener)).unwrap_or(rec)
        });
        let trap = self.trap_of(current.as_ref());
        let in_dialog = trap.and_then(|id| self.last_scopes.get(&id)).is_some_and(|scope| scope.policy.kind == ScopeKind::Dialog);
        let fields = in_dialog && dialog_tab == TabReach::Fields;
        let mut entries = self.entries(trap, all_controls && !fields);
        entries.retain(|entry| self.anchored_layer(entry.arrival().scope).is_none() && (!fields || entry.arrival().stop.text));
        let top = current.as_ref().and_then(|rec| rec.stop.group).map(|group| self.last().outermost(group));
        let Some(next) = cycle(&entries, current.as_ref(), top, forward) else { return Vec::new() };
        let arrival = entries[next].arrival().clone();
        self.focus_stop(arrival)
    }

    /// F6: the first stop of the next pane, with its label as the announcement. Inside a trapping scope (a modal
    /// dialog or a popup, also when nothing in it is focused) F6 moves nothing and announces nothing: a pane move
    /// would carry the focus out of the trap, which only closing the dialog may do.
    pub(super) fn pane(&mut self, forward: bool, all_controls: bool) -> (Vec<Act>, Vec<String>) {
        let current = self.stop_of_focus();
        if self.trap_of(current.as_ref()).is_some() {
            return (Vec::new(), Vec::new());
        }
        let panes: Vec<Id> = self
            .last_scope_order
            .iter()
            .copied()
            .filter(|id| self.last_scopes.get(id).is_some_and(|scope| scope.policy.kind == ScopeKind::Pane))
            .collect();
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
            FocusTarget::Doc(doc) => match self.docs.get(&doc).or_else(|| self.last_docs.get(&doc)) {
                Some(id) => (*id, None),
                None => return Vec::new(),
            },
        };
        // A request for a part lands on that part, never on another stop of its component: a part first declared in
        // this pass registers its stop after `update` asks for it, and then takes the focus (the awaited stop below).
        let found = self.find_stop(key).or_else(|| match key.1 {
            Some(_) => None,
            None => self.stops.iter().chain(&self.last_stops).find(|rec| rec.stop.id == key.0).cloned(),
        });
        match found {
            Some(rec) => self.focus_stop(rec),
            // An app's own widget, or a stop that registers later in this pass, which then takes the focus.
            None => {
                self.revealed = Some(key);
                self.awaited = Some(key);
                self.focus_now = Some(key.0);
                vec![Act::Focus(key.0, Default::default())]
            }
        }
    }

    /// A roving move of the group of `from`: focus moves to the member the navigation core picks. A one-line text
    /// stop in the body of a group that moves with Up and Down only, which it did not join, hands the group those
    /// keys: they enter the group at its first or last member. A move a nested group does not take passes to its
    /// parent, which steps from the nested group as one member and enters a nested group at its arrival member.
    fn roving_move(&mut self, from: StopKey, mv: NavMove, mods: MoveMods, horizontal: bool) -> Option<Vec<Act>> {
        let current = self.find_stop(from)?;
        let (mut group, member) = match current.stop.group {
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
        let along = |group: &super::Group| match (horizontal && group.rtl, mv) {
            (true, NavMove::Back) => NavMove::Forward,
            (true, NavMove::Forward) => NavMove::Back,
            (_, mv) => mv,
        };
        // The nested group the move passed out of, which is the member it starts from in `group`.
        let mut inner = None;
        for _ in 0..MAX_NESTING {
            if group.roving.takes(along(&group), mods) {
                break;
            }
            let parent = self.group(group.parent?)?;
            inner = Some(group.id);
            group = parent;
        }
        let mv = along(&group);
        if !group.roving.takes(mv, mods) {
            return None;
        }
        let members = self.last().slots(group.id);
        let at = match inner {
            Some(inner) => members.iter().position(|slot| matches!(slot, Slot::Group(id) if *id == inner)),
            // A lead that names a component whose parts form a nested group (a menu's active row) is that group.
            None => {
                let lead = group.lead.unwrap_or(from);
                members.iter().position(|slot| match slot {
                    Slot::Stop(rec) => rec.stop.key() == lead,
                    Slot::Group(id) => self.last_stops.iter().any(|rec| rec.stop.group == Some(*id) && rec.stop.id == lead.0),
                })
            }
        };
        if group.row && mv == NavMove::Back && at == Some(0) {
            return Some(self.row_owner(&group).map(|rec| self.focus_stop(rec)).unwrap_or_default());
        }
        let mut state = group.roving.state(at);
        match step(group.roving.shape, &Members { len: members.len(), first_column: group.roving.first_column }, &mut state, mv, mods) {
            NavStep::Moved(to) => {
                let arrival = match &members[to] {
                    Slot::Stop(rec) => Some(rec.clone()),
                    Slot::Group(id) => self.group(*id).and_then(|nested| self.arrival(&nested)),
                };
                Some(arrival.map(|rec| self.focus_stop(rec)).unwrap_or_default())
            }
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

    /// The focus move when the focused stop was not registered this pass, or only inside a layer that plays its exit
    /// (`fading`): a removed child of a collection row hands focus back to the row, as Escape does; any other removed
    /// stop leaves focus nowhere, whatever stops are left around it.
    fn removed_focus(&mut self, focused: Option<Id>) -> Option<Vec<Act>> {
        let root = focused?;
        let key = *self.focused.get(&root)?;
        let live = |rec: &&StopRec| !self.fading(rec.scope);
        if self.stops.iter().filter(live).any(|rec| rec.stop.key() == key) {
            return None;
        }
        let gone = self.find_stop(key)?;
        let row = gone.stop.group.and_then(|group| self.group(group)).filter(|group| group.row);
        let owner =
            row.and_then(|group| self.row_owner(&group)).filter(|owner| self.stops.iter().filter(live).any(|rec| rec.stop.key() == owner.stop.key()));
        // A removed stop in an arena (an editor or control in a canvas body) hands focus back to the arena root's own
        // stop (the canvas).
        let target = owner.or_else(|| {
            gone.stop.arena.and_then(|arena| self.stops.iter().filter(live).find(|rec| rec.stop.id == arena && rec.stop.part.is_none()).cloned())
        });
        Some(match target {
            Some(target) => self.focus_stop_from(target, FocusOrigin::Removal),
            None => vec![Act::Surrender(root)],
        })
    }

    /// Whether the scope `scope` of this pass, or one around it, is a layer playing its exit: its stops take no focus.
    fn fading(&self, scope: Option<Id>) -> bool {
        let mut at = scope;
        while let Some(rec) = at.and_then(|id| self.scopes.get(&id)) {
            if rec.closing {
                return true;
            }
            at = rec.parent;
        }
        false
    }

    /// The opener of a scope that went away, or plays its exit, while focus was inside it.
    pub(super) fn restored_focus(&self, focused: Option<Id>) -> Option<StopRec> {
        let root = focused?;
        let key = *self.focused.get(&root).unwrap_or(&(root, None));
        let inside = self.last_stops.iter().find(|rec| rec.stop.key() == key)?;
        let opener = self.opener_back(inside.scope, |id| self.scopes.get(&id).is_none_or(|now| now.closing))?;
        self.stops.iter().find(|rec| rec.stop.key() == opener).cloned()
    }

    /// Where a pick returns focus before its command runs: the pick closes the menus the walk out of the layer it ran
    /// in crosses (`walk_out`), or the modal layer it was picked in, so focus goes to the opener of the outermost scope
    /// that closes, or to the stop a menu bar took focus from when that opener is the bar. `None` when focus is in
    /// nothing the pick closes.
    pub(super) fn picked_focus(&mut self, picked: PickedIn) -> Option<StopRec> {
        let inside = self.stop_of_focus()?;
        let mut opener = match picked {
            PickedIn::Menus(layer) => {
                let walk = self.walk_out(layer);
                if !walk.crossed.iter().any(|menu| self.within(inside.scope, *menu)) {
                    return None;
                }
                walk.to?.stop.key()
            }
            PickedIn::Layer(layer) => self.opener_back(inside.scope, |id| id == layer)?,
        };
        if let Some((bar, held)) = self.bar_return
            && opener.0 == bar
        {
            self.bar_return = None;
            opener = held;
        }
        self.find_stop(opener)
    }

    /// Whether the scope `from` is `scope` or sits inside it.
    pub(super) fn within(&self, from: Option<Id>, scope: Id) -> bool {
        let mut at = from;
        while let Some(id) = at {
            if id == scope {
                return true;
            }
            at = self.scopes.get(&id).or_else(|| self.last_scopes.get(&id)).and_then(|record| record.parent);
        }
        false
    }

    /// The innermost menu scope around the scope `from`, `from` included.
    /// The container that runs the menu level the focused stop sits in: the owner of the innermost body around the
    /// stop, while it is inside a menu.
    pub(super) fn focused_menu_level(&self) -> Option<Id> {
        let rec = self.stop_of_focus()?;
        self.menu_scope(rec.scope)?;
        let body = rec.stop.body?;
        self.groups.get(&body).or_else(|| self.last_groups.get(&body)).and_then(|group| group.owner)
    }

    pub(super) fn menu_scope(&self, from: Option<Id>) -> Option<Id> {
        let mut at = from;
        while let Some(id) = at {
            let scope = self.scopes.get(&id).or_else(|| self.last_scopes.get(&id))?;
            if scope.policy.kind == ScopeKind::Menu {
                return Some(id);
            }
            at = scope.parent;
        }
        None
    }

    /// The menu `rec` is a row of: it sits in that menu and joined the roving group of the body it is in. A field in
    /// a menu (Help's search) sits in the body without joining it.
    fn row_menu(&self, rec: &StopRec) -> Option<Id> {
        (rec.stop.group.is_some() && rec.stop.group == rec.stop.body).then(|| self.menu_scope(rec.scope)).flatten()
    }

    /// Where focus goes once the menus around the scope `from` close, and the outermost of them: the walk out of the
    /// innermost menu around `from` (`walk_out`). `None` when `from` is in no menu, or the walk finds no stop.
    pub(super) fn out_of_menus(&self, from: Option<Id>) -> Option<(StopRec, Id)> {
        let walk = self.walk_out(self.menu_scope(from)?);
        let root = *walk.crossed.last()?;
        Some((walk.to?, root))
    }

    /// The walk out of the layer `layer`: it crosses `layer`, then each menu whose row the last crossed scope returns
    /// focus to (a submenu returns to its parent row), and ends at the first stop that is not a menu row, a field
    /// inside another menu included. A popup opened from such a stop starts its own chain. Which menus an activation
    /// closes, where its command resolves and fires, and where focus returns all come from this walk.
    pub(super) fn walk_out(&self, layer: Id) -> MenuWalk {
        let mut crossed = vec![layer];
        loop {
            let Some(rec) = crossed.last().and_then(|scope| self.return_stop(*scope)) else {
                return MenuWalk { to: None, crossed };
            };
            let Some(menu) = self.row_menu(&rec) else {
                return MenuWalk { to: Some(rec), crossed };
            };
            if crossed.contains(&menu) {
                return MenuWalk { to: None, crossed };
            }
            crossed.push(menu);
        }
    }

    /// The stop focus returns to when the scope `scope` closes. A scope opened while nothing held focus records its own
    /// first stop as its opener: that is no stop to return to.
    pub(super) fn return_stop(&self, scope: Id) -> Option<StopRec> {
        let rec = self.opener_back(Some(scope), |id| id == scope).and_then(|key| self.find_stop(key))?;
        (!self.within(rec.scope, scope)).then_some(rec)
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

/// Whether `id` is the app's document canvas, as the pass that registered `stops` declared it (`host_canvas`).
pub(super) fn declared_canvas(stops: &[StopRec], id: Id) -> bool {
    stops.iter().any(|rec| rec.stop.host && !rec.stop.automatic() && rec.stop.root == id)
}

/// Focus on the egui widget of `stop`, with the key filter its keys ask for.
pub(super) fn focus_act(stop: &Stop) -> Act {
    let roving = stop.group.is_some();
    let keys = stop.keys;
    // The app's canvas (`host_canvas`) reads its Escape from egui, as a stop that claims it does: egui keeps its focus
    // on it. Every other widget of the app's own keeps Escape by its own egui filter.
    let escape = (stop.host && !stop.automatic()) || keys.claims.iter().any(|chord| chord.key == craft_keymap::Key::Escape);
    let filter = egui_focus::filter(
        keys.keep == super::KeepKeys::Tab,
        roving || matches!(keys.keep, super::KeepKeys::InlineArrows | super::KeepKeys::BlockArrows),
        roving || keys.keep == super::KeepKeys::BlockArrows,
        escape,
    );
    Act::Focus(stop.root, filter)
}

fn cycle(entries: &[Entry], current: Option<&StopRec>, top: Option<Id>, forward: bool) -> Option<usize> {
    if entries.is_empty() {
        return None;
    }
    let at = current.and_then(|rec| entries.iter().position(|entry| entry.holds(rec, top)));
    Some(match (at, forward) {
        (Some(at), true) => (at + 1) % entries.len(),
        (Some(at), false) => (at + entries.len() - 1) % entries.len(),
        (None, true) => 0,
        (None, false) => entries.len() - 1,
    })
}

/// Puts the stops of the innermost open scope in Tab order: the body's, then the footer's, then the header's.
/// Inner scopes closed already and lie within one region, so they move with it. Returns how many stops the body holds,
/// now the scope's first ones; `None` when the scope marked no body.
pub(super) fn order_regions(state: &mut State) -> Option<usize> {
    let (scope, first) = state.open.last().copied()?;
    let body_end = state.body_end.remove(&scope);
    let (body, _) = state.body_from.remove(&scope)?;
    let end = state.stops.len();
    let first = first.min(end);
    let body = body.clamp(first, end);
    state.stops[first..end].rotate_left(body - first);
    Some(body_end.map_or(end, |body_end| body_end.clamp(body, end)) - body)
}

/// Closes the innermost open scope; a scope shown for the first time moves focus to its initial stop. `body` is how
/// many of the scope's first stops are its body's (`order_regions`), `None` for all of them.
pub(super) fn initial_focus(state: &mut State, frame: u64, body: Option<usize>) -> Vec<Act> {
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
    // A first stop in a roving group arrives at the group's arrival member, as Tab does.
    let lists = state.this();
    let entry = stops.first().map(|rec| {
        rec.stop
            .group
            .and_then(|group| lists.group(group))
            .filter(|group| !group.row)
            .and_then(|group| lists.arrival(&group))
            .unwrap_or_else(|| rec.clone())
    });
    // The body's first Tab entry, at its arrival member. The footer's and the header's buttons with no default take no
    // focus, so Enter does not answer one that was never chosen.
    let in_body = (policy.initial == Initial::FirstField)
        .then(|| {
            let body = Lists { stops: &stops[..body.unwrap_or(stops.len()).min(stops.len())], ..lists };
            body.entries(|_| true, true).into_iter().next().map(|entry| entry.arrival().clone())
        })
        .flatten();
    let pick = match policy.initial {
        Initial::First => entry.as_ref(),
        Initial::Default => stops.iter().find(|rec| rec.stop.default).or(entry.as_ref()),
        Initial::FirstField => stops.iter().find(|rec| rec.stop.text).or_else(|| stops.iter().find(|rec| rec.stop.default)).or(in_body.as_ref()),
        Initial::Key(name) => stops.iter().find(|rec| rec.stop.name == Some(name)),
        _ => None,
    };
    match pick.cloned() {
        Some(rec) => {
            state.entered = true;
            state.focus_stop_from(rec, FocusOrigin::Open)
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
        let moved = state.tab(forward, input.tab_all_controls, input.dialog_tab);
        state.press.by_keys |= !moved.is_empty();
        if let Some(from) = input.focused.filter(|_| !moved.is_empty()) {
            state.press.tabbed_from = Some((from, forward));
        }
        acts.extend(moved);
    }
    for forward in input.nav.panes {
        let (moved, said) = state.pane(forward, input.tab_all_controls);
        state.press.by_keys |= !moved.is_empty();
        acts.extend(moved);
        announcements.extend(said);
    }
    if let Some(at) = &input.press {
        state.classify_press(at);
    }
    (acts, announcements)
}

/// The end hook's focus moves, and the record the next input hook reads.
/// `on_app_widget`: `focused` is an app's own widget (`State::app_widget`).
pub(super) fn end(state: &mut State, focused: Option<Id>, on_app_widget: bool, frame: u64) -> Vec<Act> {
    // The moves below belong to the next frame's passes.
    state.frame = frame + 1;
    let mut acts = state.apply_press(focused, on_app_widget);
    // A layer that appeared in this pass took focus already, and a component that asked for it in this pass has it: the
    // press of the pass does not take it back.
    if state.entered || state.asked {
        acts.clear();
    }
    if acts.is_empty() {
        acts = match state.restored_focus(focused) {
            Some(rec) => state.focus_stop(rec),
            None => state.removed_focus(focused).unwrap_or_default(),
        };
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
        host: stop.as_ref().is_some_and(|stop| stop.host),
        root: stop.as_ref().filter(|stop| stop.root != stop.id && !stop.text).map(|stop| stop.root),
        menu: state.focused_menu_level(),
        pages: stop.as_ref().is_some_and(|stop| stop.role.pages),
        keys,
        menu_bar: std::mem::take(&mut state.menu_bar),
        menu_bar_by_claim: std::mem::take(&mut state.menu_bar_by_claim),
        canvas: if stop.is_none() { state.stops.iter().find(|rec| rec.stop.keys.chrome_key).map(|rec| rec.stop.id) } else { None },
        foreign: false,
        tab_in_egui: false,
        host_focus: false,
        host_text: false,
        label_selection: false,
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
