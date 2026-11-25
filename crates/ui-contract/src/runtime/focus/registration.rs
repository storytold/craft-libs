//! The registrations of the focus service: a stop and a roving group. The types are `pub` so the testing kit can
//! name them; the module is crate-private, so no other path reaches them.

use super::{FocusPolicy, Initial, KeyUse, Roving, StopKey, with};
use crate::runtime::egui::memory as store;
use crate::runtime::memory::sub;
use craft_behavior::{KeyRole, PartRef};
use egui::{Context, Id};

/// A stop as registered at step 7 of the pipeline.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Stop {
    /// The component.
    pub id: Id,
    /// The focusable part, `None` for the component itself.
    pub part: Option<PartRef>,
    /// The egui widget that holds focus for the stop: the component, or the arena root of a member.
    pub root: Id,
    /// The component's declared keys.
    pub keys: KeyUse,
    /// What the table's rows differ by.
    pub role: KeyRole,
    /// The stop is a text or editor part, which holds a draft.
    pub text: bool,
    /// Focus reaches the stop from a press only; Tab and F6 pass over it. Read-only text is such a stop: the user
    /// focuses it to select and copy.
    pub press_only: bool,
    /// The stop is its scope's default (`Initial::Default`).
    pub default: bool,
    /// The name `Initial::Key` matches.
    pub name: Option<&'static str>,
    /// The roving group the stop belongs to.
    pub group: Option<Id>,
    /// The roving group of the innermost body the stop sits in, whether or not the stop joined it.
    pub body: Option<Id>,
    /// The arena root the stop is shown in (a canvas whose body holds it), whether or not it joined the arena: a text
    /// stop keeps its own egui widget. A removed stop hands focus back to it.
    pub arena: Option<Id>,
    /// The app's own egui widget (`focus::host_canvas`, or any focusable egui widget the app draws), which reads its
    /// keys from egui's input: the key stage leaves the presses it routes to the stop there.
    pub host: bool,
}

impl Stop {
    /// A stop of component `id` with `keys`, its own egui root.
    pub fn new(id: Id, keys: KeyUse) -> Stop {
        Stop {
            id,
            part: None,
            root: id,
            keys,
            role: KeyRole::new(),
            text: false,
            press_only: false,
            default: false,
            name: None,
            group: None,
            body: None,
            arena: None,
            host: false,
        }
    }

    /// A stop on a part of the component.
    pub fn part(mut self, part: PartRef) -> Self {
        self.part = Some(part);
        self
    }

    /// A stop whose focus the arena root `root` holds.
    pub fn root(mut self, root: Id) -> Self {
        self.root = root;
        self
    }

    /// The table role of the stop.
    pub fn role(mut self, role: KeyRole) -> Self {
        self.role = role;
        self
    }

    /// Marks the stop as a text part.
    pub fn text(mut self) -> Self {
        self.text = true;
        self
    }

    /// Marks the stop as one that focus reaches from a press only.
    pub fn press_only(mut self) -> Self {
        self.press_only = true;
        self
    }

    /// Marks the stop as its scope's default.
    pub fn default_stop(mut self) -> Self {
        self.default = true;
        self
    }

    /// Names the stop for `Initial::Key`.
    pub fn named(mut self, name: &'static str) -> Self {
        self.name = Some(name);
        self
    }

    /// Joins the roving group `group`.
    pub fn in_group(mut self, group: Id) -> Self {
        self.group = Some(group);
        self
    }

    /// Sits in the body of the roving group `group`, if any.
    pub fn in_body(mut self, group: Option<Id>) -> Self {
        self.body = group;
        self
    }

    /// Sits in the arena of the root `arena` (a canvas body), if any, whether or not it joined it.
    pub fn in_arena(mut self, arena: Option<Id>) -> Self {
        self.arena = arena;
        self
    }

    /// Marks the stop as the app's own egui widget.
    pub fn host(mut self) -> Self {
        self.host = true;
        self
    }

    /// Whether this is the stop every focusable egui widget the app draws gets by itself (`focus::take_app_widgets`):
    /// the app's own widget, not declared as its canvas (`KeyUse::chrome_key`). No component or app call registered it.
    pub fn automatic(&self) -> bool {
        self.host && !self.keys.chrome_key
    }

    pub(crate) fn key(&self) -> StopKey {
        (self.id, self.part)
    }
}

/// A roving group's registration: one Tab stop whose members take moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Group {
    /// The group.
    pub id: Id,
    /// Its rules.
    pub roving: Roving,
    /// The member Tab arrives at, else the first.
    pub selected: Option<StopKey>,
    /// The member moves start from, in place of the focused one.
    pub lead: Option<StopKey>,
    /// The group is a collection row's children: Tab leaves the collection and Forward enters the row.
    pub row: bool,
    /// The component whose body or rows the group is: the container of a body group, or the collection of a row group,
    /// to which Escape, Up and Down and Back from the first child return focus.
    pub owner: Option<Id>,
    /// The group lays out right to left: a physical Left or Right arrow moves the opposite way.
    pub rtl: bool,
    /// The group this one is a member of: a move this group does not take passes to it, entering at its selected or
    /// first member.
    pub parent: Option<Id>,
}

impl Group {
    /// A group of `roving` rules.
    pub fn new(id: Id, roving: Roving) -> Group {
        Group { id, roving, selected: None, lead: None, row: false, owner: None, rtl: false, parent: None }
    }

    /// Makes this group a member of the roving group `parent`.
    #[must_use]
    pub fn parent(mut self, parent: Id) -> Self {
        self.parent = Some(parent);
        self
    }

    /// Reads horizontal arrows in the group's direction.
    pub fn rtl(mut self, rtl: bool) -> Self {
        self.rtl = rtl;
        self
    }

    /// The member Tab arrives at: a stop key, so a part of a component can be the arrival.
    pub fn selected(mut self, member: StopKey) -> Self {
        self.selected = Some(member);
        self
    }

    /// The member moves start from, in place of the focused one.
    pub fn lead(mut self, member: StopKey) -> Self {
        self.lead = Some(member);
        self
    }

    /// Marks the children of a row of `owner`'s collection.
    pub fn row(self, owner: Id) -> Self {
        Group { row: true, ..self.owner(owner) }
    }

    /// The component whose body or rows the group is.
    pub fn owner(mut self, owner: Id) -> Self {
        self.owner = Some(owner);
        self
    }
}

/// The innermost open `KeepAnchor` layer opened from the stop `opener`, the collection the layer names for its
/// opener's list keys (`FocusPolicy::list`) and the layer's policy. `None` while no such layer is open, or it names no
/// collection or one that registered no stop in the last pass (a list not shown leaves the keys with the opener). A
/// layer playing its exit fade is closed: the opener keeps its keys, so an Enter right after Escape opens the list
/// again instead of activating the row of the list that is leaving.
pub(crate) fn dropdown_target(ctx: &Context, opener: StopKey) -> Option<(Id, Id, FocusPolicy)> {
    with(ctx, |state| {
        let layer = *state.last_scope_order.iter().rev().find(|id| {
            state
                .last_scopes
                .get(*id)
                .is_some_and(|scope| scope.policy.initial == Initial::KeepAnchor && !scope.closing && scope.opener == Some(opener))
        })?;
        let policy = state.last_scopes.get(&layer)?.policy;
        let shown = |list: &Id| state.last_stops.iter().any(|rec| rec.stop.id == *list);
        policy.list.filter(shown).map(|list| (layer, list, policy))
    })
}

/// What a text stop hands to the collection it drives: set by the field each pass it is shown.
#[derive(Clone, Copy)]
pub(crate) struct Drive {
    /// The collection that takes the field's list keys.
    pub(crate) list: Id,
    /// The field holds no text, so Home, End and, for a tree, Left and Right leave the caret and go to the collection.
    pub(crate) empty: bool,
    pass: u64,
}

fn drive_slot(field: Id) -> Id {
    sub(field, "drives")
}

/// Records that the text field `field` drives the collection `list` in this pass.
#[doc(hidden)]
pub fn drive(ctx: &Context, field: Id, list: Id, empty: bool) {
    let slot = drive_slot(field);
    store::put(ctx, slot, Drive { list, empty, pass: crate::runtime::pointer::pass_of(ctx) });
    crate::runtime::memory::adopt::<Drive>(ctx, field, slot);
}

/// The collection the text field `field` drove in this pass or the one before.
pub(crate) fn driven(ctx: &Context, field: Id) -> Option<Drive> {
    let drive = store::peek_clone::<Drive>(ctx, drive_slot(field))?;
    (drive.pass.saturating_add(1) >= crate::runtime::pointer::pass_of(ctx)).then_some(drive)
}

/// Whether a collection navigates as a tree, and the pass that recorded it.
#[derive(Clone, Copy)]
struct TreeFact {
    tree: bool,
    pass: u64,
}

fn tree_slot(collection: Id) -> Id {
    sub(collection, "nav_tree")
}

/// Records whether the collection `collection` navigates as a tree in this pass: the item step writes it from its
/// layout, and the key router reads it for the text field that drives the collection.
pub(crate) fn record_tree(ctx: &Context, collection: Id, tree: bool) {
    let slot = tree_slot(collection);
    store::put(ctx, slot, TreeFact { tree, pass: crate::runtime::pointer::pass_of(ctx) });
    crate::runtime::memory::adopt::<TreeFact>(ctx, collection, slot);
}

/// Whether the collection `collection` navigated as a tree in this pass or the one before.
pub(crate) fn is_tree(ctx: &Context, collection: Id) -> bool {
    store::peek_clone::<TreeFact>(ctx, tree_slot(collection))
        .is_some_and(|fact| fact.tree && fact.pass.saturating_add(1) >= crate::runtime::pointer::pass_of(ctx))
}
