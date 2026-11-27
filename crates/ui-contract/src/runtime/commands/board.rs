//! The board: state by `(command, instance)`, the instances of each scope, the queue, resolution, firing and the
//! dispatcher that turns the key stage's shortcut events into fired commands (spec 11.3).

use super::{
    AppCatalog, CommandChain, CommandGate, CommandInput, CommandScope, CommandSet, CommandState, EditCommands, FirePhase, FireSource, Fired,
    ResolvedCommand, WindowShortcut, registry, yields_default,
};
use crate::contract::Revision;
use crate::runtime::egui::sizing::{Rerun, request_discard};
use crate::runtime::egui::{pass, repaint};
use crate::runtime::focus::EditSet;
#[cfg(feature = "testing")]
use crate::runtime::measure::Resolve;
use crate::runtime::pass_state;
use crate::runtime::reads::{self, BindKey};
use crate::runtime::{focus, frame, keys, lazy, measure, overlay, replay, sampler};
use craft_behavior::{EditCommand, KeyOwner, OwnerClass, pick_instance, resolve_flags};
use craft_keymap::{
    Active, Chord, CommandDef, CommandKind, CommandRef, Hit, KeyInput, Keymap, Layout, Matcher, ModalRule, Overrides, Platform, Press, PressKind,
    Problem, Scopes, Trigger, TypingRule,
};
use egui::{Context, Id, ViewportId};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

type StateKey = (u32, Option<Id>);
/// The key of an app-scope state: the command number and, for a ref with params, its `CommandRef::key`.
type AppStateKey = (u32, Option<String>);

/// How a command with `ModalRule::Declared` reaches into the top modal layer.
#[derive(Clone, Copy)]
enum Declaration {
    /// An instance publishes the command's state.
    Instance(Id),
    /// The focused component takes the command as an edit; `inactive` when the edit does nothing now
    /// (`KeyUse::inactive`), so the command resolves disabled.
    Focused { inactive: bool },
}

/// The registered instances in registration order, which `pick_instance` ranks by, with their positions by id.
#[derive(Default)]
struct Instances {
    list: Vec<Instance>,
    at: HashMap<Id, usize>,
}

impl Instances {
    fn get(&self, id: Id) -> Option<&Instance> {
        self.at.get(&id).map(|at| &self.list[*at])
    }

    fn iter(&self) -> std::slice::Iter<'_, Instance> {
        self.list.iter()
    }

    /// Registers `instance`, keeping the place of one already registered under the same id. Returns whether a
    /// resolution can see the difference: the id is new or was dead, or its scope, viewport or layer changed.
    fn upsert(&mut self, instance: Instance) -> bool {
        match self.at.get(&instance.id) {
            Some(at) => {
                let held = &self.list[*at];
                // A held instance that missed a pass is dead until it registers again: its return is seen too.
                let seen = held.stamp + 1 < instance.stamp
                    || held.scope != instance.scope
                    || held.viewport != instance.viewport
                    || held.layer != instance.layer;
                self.list[*at] = instance;
                seen
            }
            None => {
                self.at.insert(instance.id, self.list.len());
                self.list.push(instance);
                true
            }
        }
    }

    /// Keeps the instances `keep` accepts, in their order.
    fn retain(&mut self, mut keep: impl FnMut(&Instance) -> bool) {
        self.list.retain(|instance| keep(instance));
        self.at = self.list.iter().enumerate().map(|(at, instance)| (instance.id, at)).collect();
    }

    fn contains(&self, id: Id) -> bool {
        self.at.contains_key(&id)
    }
}

/// A container instance that receives scoped commands.
struct Instance {
    id: Id,
    scope: CommandScope,
    viewport: ViewportId,
    /// The pass of its viewport that last registered it.
    stamp: u64,
    /// The layer whose body it ran in, when it ran in one.
    layer: Option<Id>,
}

struct KeymapCache {
    keymap: Arc<Keymap>,
    platform: Platform,
    overrides: Option<Arc<Overrides>>,
    /// The generation of the app catalog the keymap joined, `None` for none.
    generation: Option<u64>,
    /// Whether hold bindings count in the keymap.
    holds: bool,
    /// The chords bound to the edit commands, which the input hook removes from the raw input when the focused
    /// component lists the command.
    edit_chords: Arc<[(EditCommand, Chord)]>,
    /// The chords bound to the window's keys, which the input hook takes before any component.
    window_shortcuts: Arc<[(WindowShortcut, Chord)]>,
    /// `BuiltKeymap::shown_text`.
    shown_text: Arc<HashMap<Chord, Arc<str>>>,
}

/// A momentary command whose key is down: its release fires the command again.
struct Held {
    key: craft_keymap::Key,
    command: CommandRef,
    target: Option<Id>,
}

/// A key down that has a hold binding: its release or its deadline settles whether it was a tap or a hold. Interaction
/// state like `Board::held`, set from a pass's own key events.
struct HoldWait {
    key: craft_keymap::Key,
    /// What the hold runs once the delay passes with the key down.
    hold: Hit,
    /// When the key went down, by its own input time.
    since: f64,
    /// The hold delay at the press: a later change of the app's delay does not move this wait.
    delay: f64,
    /// What a release before the delay runs.
    tap: Option<Hit>,
    viewport: ViewportId,
}

#[derive(Default)]
pub(crate) struct Board {
    registry: Arc<registry::Registry>,
    /// The commands the app defines at run time, from this frame's input.
    app: Option<AppCatalog>,
    keymap: Option<KeymapCache>,
    /// How many keymaps the board built: the keymap's generation (`runtime::inputs::Input::Keymap`).
    keymaps: u64,
    gate: Option<CommandGate>,
    /// The gate's answers under its current key, by command key (`CommandRef::key`). The gate is a `fn` of its props,
    /// so the board asks it once per command until the key moves, whichever readers, passes and frames resolve it.
    gate_answers: HashMap<String, bool>,
    /// Command ids by number, so a state lookup allocates nothing.
    numbers: HashMap<String, u32>,
    /// The app-scope states of the app's `Tracked<CommandStates>`, by command number, and the revision they are of.
    app_states: HashMap<AppStateKey, CommandState>,
    app_revision: Option<Revision>,
    /// The states instances published this frame and last frame; a resolution reads this frame's, else last frame's.
    this: HashMap<StateKey, CommandState>,
    last: HashMap<StateKey, CommandState>,
    /// The revision of each state key that has changed: the value of `state_seq` when it last did.
    revisions: HashMap<StateKey, u64>,
    /// Moves with every state change.
    state_seq: u64,
    /// The state keys that changed, with the `state_seq` of each change, since the oldest wake of a kept table.
    state_log: Vec<(u64, StateKey)>,
    /// The generations of the inputs every resolution reads and of the focus vantage.
    gens: Gens,
    /// Scoped commands waiting for their instance.
    queue: Vec<Fired>,
    /// App-scope commands for the next `FrameOutput`.
    out: Vec<Fired>,
    /// The latest point-bearing clipboard request in each viewport.
    pending_pastes: HashMap<ViewportId, Fired>,
    /// The app's `CommandInput::paste_reads_clipboard`.
    paste_reads_clipboard: bool,
    /// App-scope commands a key pressed while only their own state disabled them, for the next `FrameOutput`.
    declined: Vec<Fired>,
    /// The commands instances took since the last `FrameOutput`, in order.
    delivered: Vec<Fired>,
    /// The key presses since the last `FrameOutput` that ran nothing because their chord binds only disabled commands.
    dimmed: Vec<Fired>,
    instances: Instances,
    /// The focused component of each viewport's last step 12 and the instances around it.
    chains: HashMap<ViewportId, (Id, CommandChain)>,
    /// The focused component and its chain before the one in `chains`, per viewport; while the focus is in the menus
    /// or on a menu bar, the stop it returns to once they close, whose scopes a shortcut pressed there matches in.
    before: HashMap<ViewportId, (Id, CommandChain)>,
    /// Where the commands of the running pass are judged from: the key focus and the modal layers.
    here: Vantage,
    /// Each viewport's own vantage facts.
    views: HashMap<ViewportId, View>,
    matcher: Matcher,
    held: Vec<Held>,
    /// The key whose tap or hold is still to be decided.
    hold_wait: Option<HoldWait>,
    /// The app's hold delay in seconds, from this frame's input.
    hold_delay: f64,
    /// The top modal layer's root container of each viewport that has one.
    modal_roots: HashMap<ViewportId, Id>,
    /// The root of the page region the window's shell shows, per viewport: where keys with nothing focused also go.
    page_roots: HashMap<ViewportId, Id>,
    /// The states published this pass, and the pass before, by an instance inside the top modal layer.
    modal_published: HashSet<StateKey>,
    modal_published_last: HashSet<StateKey>,
    /// The canvas a modal layer keeps open (`runtime::sampler`): its scope's commands reach it while the focus is in
    /// the dialog.
    held_open: Option<Id>,
    /// Bound command ids that no set registers, already logged.
    unknown: HashSet<String>,
    /// App catalog ids that a linked set also defines, already logged.
    shadowed: HashSet<String>,
    /// What each reader was shown, by viewport (`read`).
    shown: HashMap<ViewportId, Shown>,
    /// The scope each child scope is declared inside: an instance of the child is also an instance of the parent.
    parents: HashMap<&'static str, &'static str>,
    /// The commands fired since the testing kit last took them (`observe_fired`); `None` until it first does.
    #[cfg(feature = "testing")]
    observed: Option<Vec<CommandRef>>,
}

/// Whether a resolution finds the same state under `a` and `b`: no state is the default one.
fn same_state(a: Option<&CommandState>, b: Option<&CommandState>) -> bool {
    a.cloned().unwrap_or_default() == b.cloned().unwrap_or_default()
}

/// Who read a command's resolution: the vantage it is resolved from again.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Reader {
    /// A surface with no component of its own (`resolve_global`): the key focus of the running pass.
    Global,
    /// A surface that closes before its command runs, from the window under the layer (`resolve_beneath`).
    Beneath(Id),
    /// A component, against the instances around it (`resolve_bound`).
    Chain(CommandChain),
    /// A component inside the menu `.0` (`resolve_bound`): against the innermost instance around it, judged from where
    /// focus goes once the menus close (its edits and modal facts), which also gives the instance when none is around it.
    InMenu(Id, CommandChain),
}

impl Reader {
    /// The reader's kind, for the testing check's message.
    #[cfg(feature = "testing")]
    fn kind(&self) -> &'static str {
        match self {
            Reader::Global => "global",
            Reader::Beneath(_) => "beneath-layer",
            Reader::Chain(_) => "component",
            Reader::InMenu(..) => "in-menu",
        }
    }

    /// The layer or menu whose vantage this reader is judged from, which the reader goes with when it closes.
    fn layer(&self) -> Option<Id> {
        match self {
            Reader::Beneath(layer) | Reader::InMenu(layer, _) => Some(*layer),
            _ => None,
        }
    }
}

/// The generations of what resolutions read beside states and the gate, across the viewports.
#[derive(Clone, Copy, Default, PartialEq)]
struct Gens {
    /// Read by every resolution: the keymap, the app catalog and the registry, the scope parents, the instances and
    /// their liveness, and the modal facts (open layers, their roots, the states published inside them, the held-open
    /// canvas).
    shared: u64,
    /// Read by a resolution that picks an instance: the instances around the focus in every viewport.
    focus: u64,
}

/// The generations of one viewport's own vantage facts.
#[derive(Clone, Copy, Default, PartialEq)]
struct ViewGens {
    /// Read by every resolution in the viewport: a dialog owns the keys at its focus.
    keys: u64,
    /// Read by a resolution that picks an instance or reaches by focus: the chain last focused in the viewport and the
    /// instances of its focus chain inside the top modal layer.
    focus: u64,
    /// Read by a resolution the focused stop answered: the edits that stop takes and those of them that do nothing now.
    edits: u64,
}

/// One viewport's vantage facts as its last `begin` set them, and their generations. A viewport's `begin` compares
/// with its own, so two windows never move each other's.
#[derive(Default)]
struct View {
    /// The chain of the last pass in which any component of the viewport held key focus.
    last_focused: Vec<Id>,
    inside_modal: Vec<Id>,
    dialog_keys: bool,
    /// The focused stop's edits and those of them that do nothing now. The focus record changes only in an end hook,
    /// so the value `begin` reads holds for the pass.
    edits: Option<(EditSet, EditSet)>,
    gens: ViewGens,
}
/// A press's double-press matcher, its time, and whether a binding's command accepts the press where it lands.
type ChordMatch<'a> = (&'a mut Matcher, f64, bool, &'a dyn Fn(&CommandRef) -> bool);

/// What a resolution read besides the board's own generation and the gate.
#[derive(Default)]
struct Reads {
    /// It picked an instance for a source with no component of its own, or reached by the focus.
    focus: bool,
    /// The focused stop answered for the command, or was asked: it read that stop's edits.
    edits: bool,
}

/// The inputs a kept resolution was made from: it holds while each is unchanged.
#[derive(Clone, PartialEq)]
struct ResolveKey {
    shared: u64,
    /// The viewport's keys generation.
    keys: u64,
    /// The focus generations, across the viewports and the viewport's own, when the resolution read the focus.
    focus: Option<(u64, u64)>,
    /// The state key read and its revision.
    state: Option<(StateKey, u64)>,
    /// The gate's key and its answer, with a gate.
    gate: Option<(u128, bool)>,
    /// The viewport's edits generation, when the resolution read the focused stop's edits.
    edits: Option<u64>,
    /// The vantage of a reader under a layer or inside a menu (`None` inside: no stop to return to).
    vantage: Option<Option<Vantage>>,
    /// The command's number, which a resolution the focused stop answered is noted under.
    number: u32,
}

impl ResolveKey {
    /// The key a region that showed the resolution read it under: the state's, else the focused stop's answer.
    fn bind(&self) -> Option<BindKey> {
        match (self.state, self.edits) {
            (Some(((number, instance), _)), _) => Some(BindKey::Command(number, instance)),
            (None, Some(_)) => Some(BindKey::FocusedEdit(self.number)),
            (None, None) => None,
        }
    }
}

/// A resolution a reader was shown, and what it was made from.
struct Kept {
    resolved: Arc<ResolvedCommand>,
    key: ResolveKey,
}

/// The inputs as a viewport's last wake saw them.
#[derive(Clone, Copy, Default, PartialEq)]
struct Woke {
    gens: Gens,
    view: ViewGens,
    state_seq: u64,
    gate: Option<u128>,
}

/// A kept resolution and who it was kept for.
struct Entry {
    reader: Reader,
    kept: Kept,
}

/// What each reader of a viewport was shown, with indexes from each input to the entries that read it, so a wake walks
/// only the entries whose inputs moved.
#[derive(Default)]
struct Shown {
    entries: HashMap<u64, Entry>,
    next: u64,
    /// By reader and command number (a command's parameter variants share the number).
    by_reader: HashMap<Reader, HashMap<u32, Vec<u64>>>,
    /// By the state key read.
    by_state: HashMap<StateKey, HashSet<u64>>,
    /// The entries that read the focus generations.
    by_focus: HashSet<u64>,
    /// The entries that read the focused stop's edits.
    by_edits: HashSet<u64>,
    /// The entries of readers under a layer or inside a menu, whose vantage every wake reads again.
    layered: HashSet<u64>,
    /// The inputs at the last wake.
    woke: Option<Woke>,
}

impl Shown {
    fn index(&mut self, id: u64) {
        let Some(entry) = self.entries.get(&id) else { return };
        let key = &entry.kept.key;
        if let Some((state, _)) = key.state {
            self.by_state.entry(state).or_default().insert(id);
        }
        if key.focus.is_some() {
            self.by_focus.insert(id);
        }
        if key.edits.is_some() {
            self.by_edits.insert(id);
        }
        if entry.reader.layer().is_some() {
            self.layered.insert(id);
        }
    }

    fn unindex(&mut self, id: u64) {
        let Some(entry) = self.entries.get(&id) else { return };
        if let Some((state, _)) = entry.kept.key.state
            && let Some(ids) = self.by_state.get_mut(&state)
        {
            ids.remove(&id);
            if ids.is_empty() {
                self.by_state.remove(&state);
            }
        }
        self.by_focus.remove(&id);
        self.by_edits.remove(&id);
        self.layered.remove(&id);
    }

    /// The entry `reader` keeps for `command`.
    fn find(&self, reader: &Reader, number: u32, command: &CommandRef) -> Option<u64> {
        let ids = self.by_reader.get(reader)?.get(&number)?;
        ids.iter().copied().find(|id| self.entries.get(id).is_some_and(|entry| entry.kept.resolved.command == *command))
    }

    /// Keeps `kept` for `reader`, in place of what it kept for the same command.
    fn keep(&mut self, reader: Reader, number: u32, kept: Kept) {
        match self.find(&reader, number, &kept.resolved.command) {
            Some(id) => self.replace(id, kept),
            None => {
                let id = self.next;
                self.next += 1;
                self.by_reader.entry(reader.clone()).or_default().entry(number).or_default().push(id);
                self.entries.insert(id, Entry { reader, kept });
                self.index(id);
            }
        }
    }

    /// `kept` in place of entry `id`'s, indexed under its inputs.
    fn replace(&mut self, id: u64, kept: Kept) {
        self.unindex(id);
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.kept = kept;
        }
        self.index(id);
    }

    /// Drops the entries `drop` names.
    fn drop_where(&mut self, mut drop: impl FnMut(&Entry) -> bool) {
        let gone: Vec<u64> = self.entries.iter().filter(|(_, entry)| drop(entry)).map(|(id, _)| *id).collect();
        for id in gone {
            self.unindex(id);
            if let Some(entry) = self.entries.remove(&id) {
                let number = entry.kept.key.number;
                if let Some(by_number) = self.by_reader.get_mut(&entry.reader) {
                    if let Some(ids) = by_number.get_mut(&number) {
                        ids.retain(|at| *at != id);
                        if ids.is_empty() {
                            by_number.remove(&number);
                        }
                    }
                    if by_number.is_empty() {
                        self.by_reader.remove(&entry.reader);
                    }
                }
            }
        }
    }
}

/// Whether `key` still holds under the board's `gens`, the viewport's `view` generations, `revisions`, the current
/// `gate` answer and the reader's `vantage`. A gate whose key moved but whose answer is the same keeps the resolution,
/// under the new key.
fn holds(
    key: &mut ResolveKey,
    gens: Gens,
    view: ViewGens,
    revisions: &HashMap<StateKey, u64>,
    gate: Option<(u128, bool)>,
    vantage: Option<&Option<Vantage>>,
) -> bool {
    let same = key.shared == gens.shared
        && key.keys == view.keys
        && key.focus.is_none_or(|focus| focus == (gens.focus, view.focus))
        && key.edits.is_none_or(|edits| edits == view.edits)
        && key.state.is_none_or(|(state, revision)| revisions.get(&state).copied().unwrap_or(0) == revision)
        && key.vantage.as_ref() == vantage;
    let gate = match (key.gate, gate) {
        (kept, now) if kept == now => true,
        (Some((_, kept)), Some((now, answer))) if kept == answer => {
            key.gate = Some((now, answer));
            true
        }
        _ => false,
    };
    same && gate
}

/// The edits of the stop that holds key focus in `viewport`, and those of them that do nothing now.
fn focused_edits(ctx: &Context, viewport: ViewportId) -> Option<(EditSet, EditSet)> {
    focus::record(ctx, viewport).keys.map(|keys| (keys.edits, keys.inactive))
}

/// Notes, in the read record, that the running code showed a resolution under `key` (a state's, or the focused stop's
/// answer): a settled memoized view that showed it runs when `begin` resolves the command from the same reader to something
/// else (a state, the gate, a modal layer, an instance or the stop's edits changed), and a change published after it in
/// the pass runs the pass again.
fn note_read(ctx: &Context, key: Option<BindKey>) {
    let Some(key) = key else { return };
    reads::note(ctx, key, Id::NULL, measure::active(ctx), &[]);
}

/// Where a resolution judges a command from: the modal facts and the focus. The board's own is the key focus of the
/// running pass; a surface that closes before its command runs (the palette) judges from the window under it.
#[derive(Clone, Default, PartialEq)]
struct Vantage {
    /// A modal layer blocks a window.
    modal_open: bool,
    /// A dialog owns the keys at the focus.
    dialog_keys: bool,
    /// The instances around the focus from the root of the modal layer the focus is in to the innermost: the instances
    /// `ModalRule::Blocked` does not block for a source with no component of its own. Empty while the focus is outside
    /// every modal layer.
    inside_modal: Vec<Id>,
    /// The instances around the focus, outermost first, per viewport.
    focused: Vec<(ViewportId, Vec<Id>)>,
    /// The edit commands the focused stop takes; `None` reads them from the stop that holds key focus when the
    /// command resolves.
    edits: Option<EditSet>,
    /// Those of `edits` that do nothing now, read with them.
    inactive: Option<EditSet>,
}

fn with<R>(ctx: &Context, f: impl FnOnce(&mut Board) -> R) -> R {
    pass_state::with::<Board, _>(ctx, f)
}

/// What a reader's lookup found.
enum Lookup {
    /// A kept resolution whose inputs are unchanged, and the key it was read under.
    Fresh(Arc<ResolvedCommand>, Option<BindKey>),
    /// A kept resolution some input of which changed.
    Stale,
    /// None.
    Missing,
}

impl Board {
    /// Notes that the state under `key` changed: the readers of it resolve again.
    fn bump_state(&mut self, key: StateKey) {
        self.state_seq += 1;
        self.revisions.insert(key, self.state_seq);
        self.state_log.push((self.state_seq, key));
    }

    /// The gate's key and its answer for `command`, asking it once per command under each key.
    fn gate_answer(&mut self, command: &CommandRef) -> Option<(u128, bool)> {
        let gate = self.gate.as_ref()?;
        let answer = match self.gate_answers.get(command.key_text().as_ref()) {
            Some(answer) => *answer,
            None => {
                let answer = gate.allows(command);
                self.gate_answers.insert(command.key(), answer);
                answer
            }
        };
        Some((gate.key(), answer))
    }

    /// The generations of `viewport`'s own vantage facts.
    fn view_gens(&self, viewport: ViewportId) -> ViewGens {
        self.views.get(&viewport).map_or_else(ViewGens::default, |view| view.gens)
    }

    /// What `reader` of `viewport` was shown of `command`, judged from `vantage` (a layered reader's). `edits` is the
    /// focused stop's edits now: a resolution that read them is stale once an end hook moved them, before the next
    /// `begin` moves the edits generation (a read between passes).
    fn lookup(
        &mut self,
        viewport: ViewportId,
        reader: &Reader,
        command: &CommandRef,
        vantage: Option<&Option<Vantage>>,
        edits: Option<(EditSet, EditSet)>,
    ) -> Lookup {
        let Some(&number) = self.numbers.get(command.id.as_str()) else { return Lookup::Missing };
        let gate = self.gate_answer(command);
        let view = self.view_gens(viewport);
        let edits_moved = self.views.get(&viewport).is_some_and(|kept| kept.edits != edits);
        let Some(shown) = self.shown.get_mut(&viewport) else { return Lookup::Missing };
        let Some(id) = shown.find(reader, number, command) else { return Lookup::Missing };
        let Some(entry) = shown.entries.get_mut(&id) else { return Lookup::Missing };
        let edits_hold = !edits_moved || entry.kept.key.edits.is_none();
        if holds(&mut entry.kept.key, self.gens, view, &self.revisions, gate, vantage) && edits_hold {
            Lookup::Fresh(Arc::clone(&entry.kept.resolved), entry.kept.key.bind())
        } else {
            Lookup::Stale
        }
    }

    /// Keeps `resolved`, made from `key`, as what `reader` of `viewport` was shown.
    fn keep(&mut self, viewport: ViewportId, reader: Reader, resolved: Arc<ResolvedCommand>, key: ResolveKey) {
        let number = key.number;
        self.shown.entry(viewport).or_default().keep(reader, number, Kept { resolved, key });
    }

    fn live(&self, ctx: &Context, instance: &Instance) -> bool {
        instance.stamp + 1 >= pass::pass_nr(ctx, instance.viewport)
    }

    fn live_instances(&self, ctx: &Context, scope: &str) -> Vec<Id> {
        self.instances.iter().filter(|i| self.within(i.scope.name(), scope) && self.live(ctx, i)).map(|i| i.id).collect()
    }

    /// Whether an instance of `scope` is also an instance of `ancestor`: they are one scope, or `ancestor` is among
    /// the scopes `scope` was declared a child of.
    fn within(&self, scope: &str, ancestor: &str) -> bool {
        let mut at = scope;
        for _ in 0..=self.parents.len() {
            if at == ancestor {
                return true;
            }
            match self.parents.get(at) {
                Some(parent) => at = parent,
                None => return false,
            }
        }
        false
    }

    /// `scope` and the scopes it was declared a child of, innermost first.
    fn lineage(&self, scope: &'static str) -> Vec<&'static str> {
        let mut out = vec![scope];
        while let Some(parent) = self.parents.get(out[out.len() - 1]).copied().filter(|parent| !out.contains(parent)) {
            out.push(parent);
        }
        out
    }

    /// The innermost instance in `chain` that receives the commands of `scope`.
    fn innermost(&self, chain: &CommandChain, scope: &str) -> Option<Id> {
        chain.innermost_where(|of| self.within(of, scope))
    }

    /// The instance of `scope` a command routed to `at` goes to: `at` when it is an instance of the scope itself (a
    /// canvas), else the innermost instance in `around`, the chain of the context `at` was shown in.
    fn instance_around(&self, at: Id, around: &CommandChain, scope: &str) -> Option<Id> {
        let own = self.instances.get(at).is_some_and(|i| self.within(i.scope.name(), scope));
        if own { Some(at) } else { self.innermost(around, scope) }
    }

    /// The keymap and the scopes a key matches in while `chain` (outermost first) has the keys.
    fn key_scopes(&self, chain: &[Id]) -> Option<KeyScopes> {
        let keymap = self.keymap.as_ref().map(|cache| Arc::clone(&cache.keymap))?;
        let scopes = self.scopes_around(chain);
        let typing = scopes.iter().copied().filter(|scope| self.registry.is_typing_scope(scope)).collect();
        Some(KeyScopes { keymap, scopes, held: self.held_scopes(), typing })
    }

    /// The scopes whose keys work now: those of the momentary commands whose key is held.
    fn held_scopes(&self) -> Vec<&'static str> {
        self.held.iter().filter_map(|held| self.registry.held_scope(&held.command.id)).collect()
    }

    /// Whether the commands of `scope` go to the application: the app scope's, and a held scope's, which no instance
    /// is ever of.
    fn to_app(&self, scope: &str) -> bool {
        is_app(scope) || self.registry.is_held_scope(scope)
    }

    fn is_live(&self, ctx: &Context, id: Id) -> bool {
        self.instances.get(id).is_some_and(|i| self.live(ctx, i))
    }

    /// The definition of `command`: the linked sets' by id, else the frame's app catalog's.
    fn def(&self, command: &CommandRef) -> Option<&CommandDef> {
        self.registry.get(&command.id).or_else(|| self.app.as_ref()?.get(command))
    }

    /// The scope name of `command`; commands no definition names belong to the application.
    fn scope_of(&self, command: &CommandRef) -> &str {
        self.def(command).map_or(CommandScope::APP.name(), |def| def.scope.as_str())
    }

    /// The app's state of `command`: the one set for its params, else the one set for its bare id.
    fn app_state(&self, number: u32, command: &CommandRef) -> Option<&CommandState> {
        params_key(command).and_then(|key| self.app_states.get(&(number, Some(key)))).or_else(|| self.app_states.get(&(number, None)))
    }

    fn state_key(&self, command: &CommandRef, instance: Option<Id>) -> Option<StateKey> {
        self.numbers.get(command.id.as_str()).map(|&number| (number, instance))
    }

    fn number(&mut self, id: &str) -> u32 {
        match self.numbers.get(id) {
            Some(&number) => number,
            None => {
                let number = u32::try_from(self.numbers.len()).unwrap_or(u32::MAX);
                self.numbers.insert(id.to_owned(), number);
                number
            }
        }
    }

    /// The instance a source with no component of its own delivers `scope`'s commands to.
    fn pick(&self, ctx: &Context, scope: &str, from: &Vantage) -> Option<Id> {
        // The held-open canvas ranks below every instance around the focus.
        let focused: Vec<Id> = self.held_open.into_iter().chain(from.focused.iter().flat_map(|(_, chain)| chain.iter().copied())).collect();
        let last = self.views.get(&pass::current_viewport(ctx)).map_or(&[][..], |view| view.last_focused.as_slice());
        pick_instance(&self.live_instances(ctx, scope), &focused, last)
    }

    /// The non-app scopes around the focus, innermost first. An instance of a child scope also brings the scopes it
    /// is declared inside, after its own.
    fn scopes_around(&self, chain: &[Id]) -> Vec<&'static str> {
        let mut scopes: Vec<&'static str> = Vec::new();
        for instance in chain.iter().rev().filter_map(|id| self.instances.get(*id)) {
            let lineage = self.lineage(instance.scope.name());
            scopes.push(lineage[0]);
            let parents: Vec<&'static str> = lineage[1..].iter().copied().filter(|scope| !scopes.contains(scope)).collect();
            scopes.extend(parents);
        }
        scopes.retain(|scope| !is_app(scope));
        scopes
    }

    /// The instances of `chain` (outermost first) inside the top modal layer: its root and the instances within it.
    /// Empty when the chain is outside every modal layer.
    fn inside_modal_of(chain: &[Id], is_root: impl Fn(Id) -> bool) -> Vec<Id> {
        chain.iter().position(|id| is_root(*id)).map_or_else(Vec::new, |at| chain[at..].to_vec())
    }

    fn is_modal_root(&self, id: Id) -> bool {
        self.modal_roots.values().any(|root| *root == id)
    }

    /// Where `command` goes when a source with no component of its own fires it: `None` for an app-scope command,
    /// else the picked instance, which may not exist.
    fn target_of(&self, ctx: &Context, command: &CommandRef, from: &Vantage) -> (bool, Option<Id>) {
        let scope = self.scope_of(command);
        let app = self.to_app(scope);
        (app, if app { None } else { self.pick(ctx, scope, from) })
    }

    /// Who answers for `command` ahead of the app: an instance inside the top modal layer that published its state,
    /// else the focused component when it takes the command as an edit (inside a modal layer, only when the focus
    /// is in it).
    fn declaring(&self, ctx: &Context, command: &CommandRef, from: &Vantage, reads: &mut Reads) -> Option<Declaration> {
        reads.focus = true;
        let dialog_open = from.modal_open || from.dialog_keys;
        let number = *self.numbers.get(command.id.as_str())?;
        let published = self.modal_published.iter().chain(&self.modal_published_last).find(|(published, _)| *published == number);
        if let Some(id) = published.and_then(|(_, id)| *id).filter(|_| dialog_open) {
            return Some(Declaration::Instance(id));
        }
        let edit = self.registry.edit_of(&command.id)?;
        let keys = match from.edits {
            Some(edits) => Some((edits, from.inactive.unwrap_or(EditSet::NONE))),
            None => {
                reads.edits = true;
                focused_edits(ctx, pass::current_viewport(ctx))
            }
        };
        let in_reach = !dialog_open || !from.inside_modal.is_empty();
        let (edits, inactive) = keys.filter(|_| in_reach)?;
        let inactive = inactive.contains(edit);
        // An Undo or Redo the focused part has no step for goes where `fall_through` sends it (the declaring instance
        // or the app command), so that target answers for it and its state enables the command: a toolbar Undo stays
        // enabled while the app can undo though the field cannot. Cut, Copy and Delete with nothing selected stay the
        // part's and disabled.
        if inactive && matches!(edit, EditCommand::Undo | EditCommand::Redo) {
            return None;
        }
        edits.contains(edit).then_some(Declaration::Focused { inactive })
    }

    /// `command` as seen by `instance`, or by the app for an app-scope command, from `from`, before the gate, and the
    /// key of the state it read (`None` when the focused component answers for the command).
    fn resolve(
        &self,
        ctx: &Context,
        command: &CommandRef,
        instance: Option<Id>,
        reach: Reach<'_>,
        from: &Vantage,
        reads: &mut Reads,
    ) -> (ResolvedCommand, Option<StateKey>) {
        let def = self.def(command);
        let app = self.to_app(self.scope_of(command));
        let instance = if app { None } else { instance };
        let dialog_open = from.modal_open || from.dialog_keys;
        // A command no set registers is never blocked.
        let rule = def.map(|def| def.modal);
        // The instance inside the top modal that declares the command, which then answers for it.
        let declaring = (rule == Some(ModalRule::Declared)).then(|| self.declaring(ctx, command, from, reads)).flatten();
        // The declaring instance answers for the command: it is where the command is delivered. The focused component
        // answers for itself, so the app's state of the command does not apply.
        let instance = match declaring {
            Some(Declaration::Instance(id)) => Some(id),
            _ => instance,
        };
        let key = match declaring {
            Some(Declaration::Instance(id)) => self.numbers.get(command.id.as_str()).map(|&number| (number, Some(id))),
            Some(Declaration::Focused { .. }) => None,
            None => self.state_key(command, instance),
        };
        // An app-scope state is the app's held one; an instance's is this frame's publish, else last frame's.
        let (this, last) = match key {
            Some((number, None)) => (self.app_state(number, command), None),
            Some(key) => (self.this.get(&key), self.last.get(&key)),
            None => (None, None),
        };
        let state = this.or(last);
        let flags = resolve_flags(this.map(CommandState::flags), last.map(CommandState::flags));
        // The focused part answers for a declared edit's flags, but its title and icon stay the app's, so the edit's
        // label does not change language with the focus (#3569). The hint stays the part's: the app's hint describes
        // the app's own enable state, which the part's flags override.
        let app_number = matches!(declaring, Some(Declaration::Focused { .. })).then(|| self.numbers.get(command.id.as_str()).copied()).flatten();
        let shown = state.or_else(|| app_number.and_then(|number| self.app_state(number, command)));
        // The resolution reads the app's held state, so a change to it invalidates the kept resolution.
        let key = key.or(app_number.map(|number| (number, None)));
        let reached = declaring.is_some()
            || instance.is_some_and(|id| match reach {
                Reach::Focus => {
                    reads.focus = true;
                    from.inside_modal.contains(&id)
                }
                Reach::Chain(chain) => chain.ids().skip_while(|at| !self.is_modal_root(*at)).any(|at| at == id),
            });
        let modal = !dialog_open
            || match rule {
                None | Some(ModalRule::Always) => true,
                // A modeless dialog that owns the keys has no modal root to reach into. The canvas a modal layer
                // holds open is the one instance outside it that its view commands reach.
                Some(ModalRule::InDialogs) => app || !from.modal_open || reached || instance.is_some_and(|id| Some(id) == self.held_open),
                // `Blocked` and `Declared`.
                Some(_) => reached,
            };
        let live = app || instance.is_some_and(|id| self.is_live(ctx, id));
        let refused = !(modal && live && !matches!(declaring, Some(Declaration::Focused { inactive: true })));
        let chord = self.keymap.as_ref().and_then(|cache| cache.keymap.shown_chord(command));
        let chord_text = chord.and_then(|chord| self.keymap.as_ref()?.shown_text.get(&chord).cloned());
        let resolved = ResolvedCommand {
            label: shown.and_then(|s| s.title_ref().cloned()).unwrap_or_else(|| Arc::from(def.map_or(command.id.as_str(), |def| def.label.as_str()))),
            catalog_label: def.map(|def| Arc::from(def.label.as_str())),
            icon: shown.and_then(|s| s.icon_ref().cloned()).or_else(|| def.and_then(|def| def.icon.clone())),
            chord,
            chord_text,
            enabled: flags.enabled && !refused,
            refused,
            checked: flags.checked,
            marked: state.is_some_and(CommandState::is_marked),
            hint: state.and_then(|s| s.hint_ref().cloned()),
            hint_alone: state.is_some_and(CommandState::is_hint_alone),
            kind: def.map_or(CommandKind::Command, |def| def.kind.clone()),
            edit: self.registry.edit_of(&command.id).is_some(),
            command: command.clone(),
            instance,
        };
        (resolved, key)
    }
}

fn is_app(scope: &str) -> bool {
    scope == CommandScope::APP.name()
}

/// The key of `command`'s app state when it carries params: `None` for a bare id, which answers every ref of its id.
fn params_key(command: &CommandRef) -> Option<String> {
    let key = command.key_text();
    (key != command.id.as_str()).then(|| key.into_owned())
}

/// The window's key matching: the keymap and the scopes a key matches in, innermost first. The key stage, the
/// routing question whether a scoped command takes a key (`binds_scoped`) and a key automation holds at a canvas
/// (`bound_around`) all match through it.
struct KeyScopes {
    keymap: Arc<Keymap>,
    scopes: Vec<&'static str>,
    /// The held scopes ([`Board::held_scopes`]): the keymap's contexts, whose keys come before every scope's.
    held: Vec<&'static str>,
    /// The typing scopes among `scopes` (`CommandSet::TYPING`): contexts of the keymap too, active while typing.
    typing: Vec<&'static str>,
}

impl KeyScopes {
    /// Runs `f` with the active scopes and the outer ones a press matches in, with `typing` when a text field has the
    /// keys. A held scope's keys stay with the text while typing; a typing scope's keys are the text component's own.
    fn matching<R>(&self, typing: bool, f: impl FnOnce(&Active<'_>, &[&str]) -> R) -> R {
        let (panel, outer) = self.scopes.split_first().map_or((None, &[][..]), |(panel, outer)| (Some(*panel), outer));
        let held: &[&'static str] = if typing { &[] } else { &self.held };
        let contexts: Vec<&str> = self.typing.iter().chain(held).copied().collect();
        f(&Active { panel, contexts: &contexts, typing }, outer)
    }

    /// What a press of `input` runs, read through `layout`. With `matcher`, the press's time and whether it is a held
    /// key's repeat, a press counts toward a double press; without, it is matched on its own. With a matcher, a binding
    /// of a command `accept` refuses (disabled where the press lands) is passed over for the press's next binding.
    fn press(&self, layout: &Layout, input: &KeyInput, typing: bool, matcher: Option<ChordMatch<'_>>) -> Option<Hit> {
        self.matching(typing, |active, outer| match matcher {
            Some((matcher, now, repeat, accept)) => matcher.press_where(layout, &self.keymap, &Press { input, repeat, now, active, outer }, accept),
            None => Matcher::single(layout, &self.keymap, input, active, outer),
        })
    }

    /// What a hold of `input`'s key runs, when a hold binding applies.
    fn hold(&self, layout: &Layout, input: &KeyInput, typing: bool) -> Option<Hit> {
        // PLAN: assumed `Matcher::hold(layout, keymap, input, active, outer) -> Option<Hit>` (kb.C13f, shortcuts-w3)
        self.matching(typing, |active, outer| Matcher::hold(layout, &self.keymap, input, active, outer))
    }
}

/// A press of `chord` known only by its key, as injected key events carry it: the typed key at its own position.
fn chord_input(chord: &Chord) -> KeyInput {
    KeyInput::new(chord.mods).with_typed(chord.key).with_physical(chord.key)
}

/// Loads the linked command sets; the stories' sets only with `stories`.
pub(crate) fn install(ctx: &Context, stories: bool) -> Result<(), registry::CommandDefError> {
    let registry = registry::Registry::shared(stories)?;
    with(ctx, |board| {
        board.registry = registry;
        board.gens.shared += 1;
    });
    Ok(())
}

/// A keymap with the chords it binds to the edit commands and to the window's keys.
#[derive(Clone)]
pub(crate) struct BuiltKeymap {
    keymap: Arc<Keymap>,
    edit_chords: Arc<[(EditCommand, Chord)]>,
    window_shortcuts: Arc<[(WindowShortcut, Chord)]>,
    /// The text of every chord the keymap shows, written once when the keymap is built.
    shown_text: Arc<HashMap<Chord, Arc<str>>>,
}

/// The keymap of `defs` on `platform` under `overrides`, with the system chords reserved, the app's typing rule when it
/// set one, and hold bindings counting when `holds`.
fn keymap_of(
    defs: Vec<CommandDef>,
    scopes: Scopes,
    platform: Platform,
    overrides: Option<&Arc<Overrides>>,
    typing: Option<TypingRule>,
    holds: bool,
) -> Keymap {
    let mut keymap = Keymap::new(defs, scopes, platform).with_reserved(craft_keymap::reserved::system(platform));
    if let Some(rule) = typing {
        keymap = keymap.with_typing_rule(rule);
    }
    keymap.set_holds(holds);
    if let Some(overrides) = overrides {
        keymap.set_overrides((**overrides).clone());
    }
    keymap
}

#[cfg(feature = "testing")]
thread_local! {
    static CHORD_TEXT_BUILDS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many chord texts this thread's keymap builds have written: one per distinct shown chord per build, never one
/// per report.
#[cfg(feature = "testing")]
#[doc(hidden)]
pub fn chord_text_builds() -> u64 {
    CHORD_TEXT_BUILDS.with(std::cell::Cell::get)
}

#[cfg(feature = "testing")]
thread_local! {
    static KEYMAP_BUILDS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many keymaps this thread has built: a frame builds one only when the platform, the overrides or the app
/// catalog's generation differ from the last built one's.
#[cfg(feature = "testing")]
#[doc(hidden)]
pub fn keymap_builds() -> u64 {
    KEYMAP_BUILDS.with(std::cell::Cell::get)
}

/// The keymap of `registry` and the app catalog `app` on `platform` under `overrides`, and the chords it binds to the
/// edit commands and to the window's keys. A catalog command whose id the registry has keeps the registry's
/// definition, and the catalog's default bindings join the registry's after them (an app's alternate keys, such as
/// F2/F3/F4 for Cut/Copy/Paste).
///
/// A yielding library command's default chord (a window shortcut's, Quit's: `yields_default`) that another command also
/// binds is that command's: the library command loses it, so the app's binding beats the framework's default. One the
/// overrides name keeps what they give it.
fn build_keymap(
    registry: &registry::Registry,
    app: Option<&AppCatalog>,
    platform: Platform,
    overrides: Option<&Arc<Overrides>>,
    holds: bool,
) -> BuiltKeymap {
    #[cfg(feature = "testing")]
    KEYMAP_BUILDS.with(|builds| builds.set(builds.get() + 1));
    // A held scope and a typing scope are contexts of the keymap: their keys work while they are active, before any
    // panel's, and a typing scope's while typing too.
    let mut scopes = Scopes::new();
    for def in registry.defs() {
        if registry.is_held_scope(def.scope.as_str()) || registry.is_typing_scope(def.scope.as_str()) {
            scopes = scopes.with_context(def.scope.as_str());
        } else if !def.scope.is_app() {
            scopes = scopes.with_panel(def.scope.as_str());
        }
    }
    let app_defs: Vec<CommandDef> =
        app.map(|app| app.defs().iter().filter(|def| registry.get(&def.command.id).is_none()).cloned().collect()).unwrap_or_default();
    // The window-shortcut yield and the edit chords below read the registry's definitions only.
    let joined = |defs: &[CommandDef]| -> Vec<CommandDef> { defs.iter().chain(&app_defs).cloned().collect() };
    let mut defs = registry.defs_for(platform);
    if let Some(app) = app {
        // The app's defaults for a linked set's command replace the set's, before the app's alternate keys join them.
        for def in &mut defs {
            if let Some(bindings) = app.replaced_defaults(&def.command.id) {
                def.defaults = bindings.to_vec();
            }
        }
        for extra in app.defs() {
            if let Some(def) = defs.iter_mut().find(|def| def.command == extra.command) {
                for binding in &extra.defaults {
                    if !def.defaults.contains(binding) {
                        def.defaults.push(binding.clone());
                    }
                }
            }
        }
    }
    let typing = app.and_then(AppCatalog::typing);
    let mut keymap = keymap_of(joined(defs.as_slice()), scopes.clone(), platform, overrides, typing, holds);
    let overridden = |def: &CommandDef| overrides.is_some_and(|overrides| overrides.get(&def.command.id).is_some());
    let yielded: Vec<(CommandRef, Trigger)> = defs
        .iter()
        .filter(|def| yields_default(&def.command) && !overridden(def))
        .flat_map(|def| keymap.conflicts_with(&def.command))
        .filter(|conflict| !yields_default(&conflict.b.0))
        .map(|conflict| (conflict.a.0, conflict.chord))
        .collect();
    if !yielded.is_empty() {
        for def in &mut defs {
            def.defaults
                .retain(|binding| !yielded.iter().any(|(command, chord)| *command == def.command && binding.trigger.effective(platform) == *chord));
        }
        keymap = keymap_of(joined(defs.as_slice()), scopes, platform, overrides, typing, holds);
    }
    // An override whose key names no command binds nothing and leaves that command's defaults in place; a misspelt id in
    // an app's own table otherwise fails without a trace.
    for problem in keymap.problems() {
        if let Problem::UnknownCommand { key } = problem {
            log::warn!("shortcut override `{key}` names no command and binds nothing");
        }
    }
    let window_shortcuts = registry
        .defs()
        .iter()
        .filter_map(|def| WindowShortcut::of(&def.command).map(|key| (key, &def.command)))
        .flat_map(|(key, command)| {
            keymap
                .bindings(command)
                .into_iter()
                .filter(|binding| binding.trigger.press == PressKind::Single)
                .map(move |binding| (key, binding.trigger.chord))
        })
        .collect();
    let edit_chords = registry
        .defs()
        .iter()
        .filter_map(|def| registry.edit_of(&def.command.id).map(|edit| (edit, &def.command)))
        // Delete's keys (Delete, Backspace) stay a focused text part's own deletion, which also deletes one character.
        .filter(|(edit, _)| *edit != EditCommand::Delete)
        .flat_map(|(edit, command)| {
            // Only the platform's own text-editing chords: an app's alternate key (F4 for Paste) is a command key,
            // which the typing rule decides on, as a text field elsewhere on the platform ignores it.
            keymap
                .bindings(command)
                .into_iter()
                .filter(|binding| {
                    binding.trigger.press == PressKind::Single
                        && craft_keymap::text_editing_chords(platform).contains(&binding.trigger.chord.effective(platform))
                })
                .map(move |binding| (edit, binding.trigger.chord))
        })
        .collect();
    let mut shown_text = HashMap::new();
    for def in keymap.catalog() {
        if let Some(chord) = keymap.shown_chord(&def.command) {
            shown_text.entry(chord).or_insert_with(|| {
                #[cfg(feature = "testing")]
                CHORD_TEXT_BUILDS.with(|builds| builds.set(builds.get() + 1));
                Arc::from(chord.display(platform))
            });
        }
    }
    BuiltKeymap { keymap: Arc::new(keymap), edit_chords, window_shortcuts, shown_text: Arc::new(shown_text) }
}

/// Builds or reuses a keymap owned by this exact command registry: one with no catalog, no overrides and holds off is
/// built once per platform.
fn keymap_for(
    registry: &Arc<registry::Registry>,
    app: Option<&AppCatalog>,
    platform: Platform,
    overrides: Option<&Arc<Overrides>>,
    holds: bool,
) -> BuiltKeymap {
    if app.is_some() || overrides.is_some() || holds {
        return build_keymap(registry, app, platform, overrides, holds);
    }
    let mut kept = registry.keymaps.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, built)) = kept.iter().find(|(on, _)| *on == platform) {
        return built.clone();
    }
    let built = build_keymap(registry, None, platform, None, false);
    kept.push((platform, built.clone()));
    built
}

/// Declares that an instance of `child` also receives the commands of `parent`, and that its shortcuts rank inside the
/// parent's. The workspace scope of a full-window dialog is a child of the view scope.
#[doc(hidden)]
pub fn declare_child(ctx: &Context, child: CommandScope, parent: CommandScope) {
    with(ctx, |board| {
        if board.parents.insert(child.name(), parent.name()) != Some(parent.name()) {
            board.gens.shared += 1;
        }
    });
}

/// The innermost instance in `chain` that receives the commands of `scope`: an instance of the scope or of a child of it.
pub(crate) fn innermost(ctx: &Context, chain: &CommandChain, scope: &str) -> Option<Id> {
    with(ctx, |board| board.innermost(chain, scope))
}

/// The platform the keymap and the chords shown are read for.
fn platform(ctx: &Context) -> Platform {
    crate::runtime::windows::os_of(ctx)
}

/// Before each frame: the states instances published last frame are the fallback, the app's states are read again
/// when their revision moved, the gate and the catalog are this frame's, and the keymap follows the user's overrides
/// and the catalog's generation. The readers of an input that changed run from `begin`, which resolves again what each
/// was shown.
pub(crate) fn set_input(ctx: &Context, input: &CommandInput) {
    let platform = platform(ctx);
    let generation = input.catalog.as_ref().map(AppCatalog::generation);
    let holds = input.holds.is_some();
    let shadowed = with(ctx, |board| {
        // A published state resolves from this frame's, else the last frame's: one published last frame and not again
        // is gone now.
        let gone: Vec<StateKey> = board.last.keys().filter(|key| !board.this.contains_key(key)).copied().collect();
        board.last = std::mem::take(&mut board.this);
        for key in gone {
            board.bump_state(key);
        }
        if board.modal_published_last.iter().any(|key| !board.modal_published.contains(key)) {
            board.gens.shared += 1;
        }
        board.modal_published_last = std::mem::take(&mut board.modal_published);
        let revision = input.states.as_ref().map(|(revision, _)| *revision);
        if revision != board.app_revision {
            board.app_revision = revision;
            let mut states = HashMap::new();
            for (command, state) in input.states.iter().flat_map(|(_, states)| states.iter()) {
                states.insert((board.number(&command.id), params_key(command)), state.clone());
            }
            let changed: Vec<u32> = states
                .iter()
                .filter(|(key, state)| board.app_states.get(*key) != Some(*state))
                .map(|(key, _)| key.0)
                .chain(board.app_states.keys().filter(|key| !states.contains_key(*key)).map(|key| key.0))
                .collect();
            board.app_states = states;
            for number in changed {
                board.bump_state((number, None));
            }
        }
        if input.gate.as_ref().map(CommandGate::key) != board.gate.as_ref().map(CommandGate::key) {
            board.gate_answers.clear();
        }
        board.gate = input.gate.clone();
        board.app = input.catalog.clone();
        board.hold_delay = input.holds.unwrap_or_default();
        // Turning holds off ends a wait: its key's hold no longer exists.
        if !holds {
            board.hold_wait = None;
        }
        board.paste_reads_clipboard = input.paste_reads_clipboard;
        let stale = board.keymap.as_ref().is_none_or(|cache| {
            cache.platform != platform
                || cache.generation != generation
                || cache.holds != holds
                || match (&cache.overrides, &input.overrides) {
                    (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                    (None, None) => false,
                    _ => true,
                }
        });
        let mut shadowed = Vec::new();
        if stale {
            let BuiltKeymap { keymap, edit_chords, window_shortcuts, shown_text } =
                keymap_for(&board.registry, input.catalog.as_ref(), platform, input.overrides.as_ref(), holds);
            board.keymap = Some(KeymapCache {
                keymap,
                platform,
                overrides: input.overrides.clone(),
                generation,
                holds,
                edit_chords,
                window_shortcuts,
                shown_text,
            });
            board.gens.shared += 1;
            board.keymaps += 1;
            for def in input.catalog.as_ref().map_or(&[][..], AppCatalog::defs) {
                if board.registry.get(&def.command.id).is_some() && board.shadowed.insert(def.command.id.clone()) {
                    shadowed.push(def.command.id.clone());
                }
            }
        }
        shadowed
    });
    for id in shadowed {
        log::warn!("the app catalog defines {id}, which a linked command set defines: the set's definition is the command's");
    }
}

/// Tells the app of a key press `hit` that ran nothing because its command `resolved` is disabled, when the app's own
/// state of an app-scope command (`app`) is all that disables it, so the app can say why nothing ran
/// (`FrameOutput::declined`). A refusal by the gate, the modal rule or a declaring instance's state is not the app's to
/// explain.
fn decline(ctx: &Context, hit: Hit, app: bool, resolved: &ResolvedCommand, time: f64) {
    if resolved.enabled || !app || resolved.instance.is_some() || resolved.refused {
        return;
    }
    let declined = Fired {
        command: hit.command,
        target: None,
        source: FireSource::Key,
        phase: FirePhase::Press,
        cycle: hit.cycle.len() > 1,
        alt_variant: hit.alt_variant,
        mods: hit.trigger.chord.mods,
        time,
        place: None,
        pasted: None,
        paste_at: None,
    };
    with(ctx, |board| board.declined.push(declined));
}

/// The chords the keymap binds to the edit commands, which the input hook removes from the raw input when the
/// focused component lists the command.
pub(crate) fn edit_chords(ctx: &Context) -> Arc<[(EditCommand, Chord)]> {
    with(ctx, |board| board.keymap.as_ref().map(|cache| Arc::clone(&cache.edit_chords)).unwrap_or_default())
}

/// The generation of the window's keymap without noting a read: it moves each time a new keymap is built.
pub(crate) fn keymap_built(ctx: &Context) -> u64 {
    with(ctx, |board| board.keymaps)
}

/// The generation of the window's keymap, for a key that holds what is derived from its chords: it moves each time a
/// new keymap is built. The caller's view reads the keymap, so a change of it runs the view.
pub fn keymap_generation(ctx: &Context) -> u64 {
    crate::runtime::replay::note_input(ctx, crate::runtime::inputs::Input::Keymap);
    keymap_built(ctx)
}

/// Whether the queue holds a command for one of `instances`: only a run of the view they registered in takes it.
pub(crate) fn queued_for(ctx: &Context, instances: &[Id]) -> bool {
    !instances.is_empty() && with(ctx, |board| board.queue.iter().any(|fired| fired.target.is_some_and(|target| instances.contains(&target))))
}

/// The bindings the window's keymap gives `command`; none before the first frame builds one.
pub(crate) fn bindings(ctx: &Context, command: &CommandRef) -> Vec<craft_keymap::Binding> {
    crate::runtime::replay::note_input(ctx, crate::runtime::inputs::Input::Keymap);
    with(ctx, |board| board.keymap.as_ref().map(|cache| cache.keymap.bindings(command)).unwrap_or_default())
}

/// The chord the window's keymap shows for `command` (`Keymap::shown_chord`); none before the first frame builds one.
pub(crate) fn chord(ctx: &Context, command: &CommandRef) -> Option<Chord> {
    crate::runtime::replay::note_input(ctx, crate::runtime::inputs::Input::Keymap);
    with(ctx, |board| board.keymap.as_ref().and_then(|cache| cache.keymap.shown_chord(command)))
}

/// The window's own keymap: the registry scopes, the app catalog, the window shortcuts and the app's overrides, as the
/// board resolves keys with it; none before the first frame builds one.
pub fn keymap(ctx: &Context) -> Option<Arc<Keymap>> {
    with(ctx, |board| board.keymap.as_ref().map(|cache| Arc::clone(&cache.keymap)))
}

/// The command that holds `chord` in the window's keymap (`Keymap::holder`); none before the first frame builds one.
pub(crate) fn holder(ctx: &Context, chord: &Chord) -> Option<CommandRef> {
    crate::runtime::replay::note_input(ctx, crate::runtime::inputs::Input::Keymap);
    with(ctx, |board| board.keymap.as_ref().and_then(|cache| cache.keymap.holder(chord)))
}

/// Whether a press of `chord` in `viewport` can run something: a command the window's keymap binds it to in a scope
/// around the focus of the viewport's last pass, matched as the key stage matches a press outside typing, is enabled as
/// that pass resolved it for a surface with no component of its own (the state a menu row shows). A binding in a scope
/// that is not around the focus (a typing scope with no editor shown) runs nothing. Notes no reader.
pub(crate) fn runs_now(ctx: &Context, viewport: ViewportId, chord: &Chord) -> bool {
    let keys = with(ctx, |board| {
        let chain = board.here.focused.iter().find(|(at, _)| *at == viewport).map(|(_, chain)| chain.clone()).unwrap_or_default();
        board.key_scopes(&chain)
    });
    let Some(keys) = keys else { return false };
    let enabled = |command: &CommandRef| resolve_from(ctx, command, None).0.enabled;
    keys.matching(false, |active, outer| !matches!(keys.keymap.lookup_where(chord, false, active, outer, &enabled), craft_keymap::Lookup::None))
}

/// The chords bound to the window's keys, which the input hook takes before any component.
pub(crate) fn window_shortcuts(ctx: &Context) -> Arc<[(WindowShortcut, Chord)]> {
    with(ctx, |board| board.keymap.as_ref().map(|cache| Arc::clone(&cache.window_shortcuts)).unwrap_or_default())
}

/// The app-scope commands fired since the last call.
pub(crate) fn take_fired(ctx: &Context) -> Vec<Fired> {
    with(ctx, |board| std::mem::take(&mut board.out))
}

/// The commands fired since the last call, whatever they were routed to, and from now on records them: the testing
/// kit's story observer takes them around each pass of a story. Nothing is recorded before the first call.
#[cfg(feature = "testing")]
pub(crate) fn observe_fired(ctx: &Context) -> Vec<CommandRef> {
    with(ctx, |board| board.observed.replace(Vec::new()).unwrap_or_default())
}

/// The app-scope commands a key press fired while only the app's own command state disabled them, since the last
/// call.
pub(crate) fn take_declined(ctx: &Context) -> Vec<Fired> {
    with(ctx, |board| std::mem::take(&mut board.declined))
}

/// The commands instances took since the last call.
pub(crate) fn take_delivered(ctx: &Context) -> Vec<Fired> {
    with(ctx, |board| std::mem::take(&mut board.delivered))
}

/// The key presses that hit only disabled commands since the last call.
pub(crate) fn take_dimmed(ctx: &Context) -> Vec<Fired> {
    with(ctx, |board| std::mem::take(&mut board.dimmed))
}

/// Whether the component that holds key focus sits inside the instance `id`.
#[doc(hidden)]
pub fn focus_within(ctx: &Context, id: Id) -> bool {
    crate::runtime::replay::note_input(ctx, crate::runtime::inputs::Input::Focus);
    with(ctx, |board| board.here.focused.iter().any(|(_, chain)| chain.contains(&id)))
}

/// Simulates a focus change after the command board's previous begin hook.
#[cfg(feature = "testing")]
#[doc(hidden)]
pub fn clear_focus_vantage(ctx: &Context) {
    with(ctx, |board| {
        board.here.inside_modal.clear();
        for (_, chain) in &mut board.here.focused {
            chain.clear();
        }
    });
}

/// Records that the container `id` is an instance of `scope` in the running pass of its viewport.
pub(crate) fn register_instance(ctx: &Context, scope: CommandScope, id: Id) {
    register_in(ctx, scope, id, overlay::drawing_parent(ctx));
}

/// `register_instance` in the layer `layer`, which a replay of the registration keeps.
fn register_in(ctx: &Context, scope: CommandScope, id: Id, layer: Option<Id>) {
    crate::runtime::replay::journal(ctx, || (scope, id, layer), |ctx, &(scope, id, layer)| register_in(ctx, scope, id, layer));
    crate::runtime::replay::note_instance(ctx, id);
    let viewport = pass::current_viewport(ctx);
    let stamp = pass::pass_nr(ctx, viewport);
    with(ctx, |board| {
        if board.instances.upsert(Instance { id, scope, viewport, stamp, layer }) {
            board.gens.shared += 1;
        }
    });
}

/// Records the instances around the component `id`, which holds key focus in the running pass of its viewport. The
/// next begin hook of the viewport reads the chain of the component its focus record names. While the focus moves
/// inside the open menus, or onto a menu bar and into its menus, the component it left stays in `before` only when
/// it is where the focus returns to (`Returning`): rows passed over, the bar and submenu openers never replace it.
pub(crate) fn note_focus(ctx: &Context, id: Id, chain: &CommandChain) {
    let viewport = pass::current_viewport(ctx);
    let moved = with(ctx, |board| board.chains.get(&viewport).is_none_or(|(held, _)| *held != id));
    let returns = moved.then(|| Returning::of(ctx, id)).flatten().map(Returning::id);
    with(ctx, |board| {
        if let Some(old) = board.chains.insert(viewport, (id, chain.clone()))
            && old.0 != id
            && returns.is_none_or(|to| to == old.0)
        {
            board.before.insert(viewport, old);
        }
    });
}

pub(crate) fn set_modal_open(ctx: &Context, open: bool, viewport: ViewportId, root: Option<Id>) {
    with(ctx, |board| {
        let held = match root {
            Some(root) => board.modal_roots.insert(viewport, root),
            None => board.modal_roots.remove(&viewport),
        };
        if board.here.modal_open != open || held != root {
            board.gens.shared += 1;
        }
        board.here.modal_open = open;
    });
}

/// Records `root` as the page region `viewport` shows (`AppShell::page`): a key pressed with nothing focused reaches
/// the page's command scope beside the window's.
#[doc(hidden)]
pub fn note_page_root(ctx: &Context, viewport: ViewportId, root: Id) {
    with(ctx, |board| {
        board.page_roots.insert(viewport, root);
    });
}

/// The root of the top modal layer of `viewport`, if one is open.
pub(crate) fn modal_root(ctx: &Context, viewport: ViewportId) -> Option<Id> {
    with(ctx, |board| board.modal_roots.get(&viewport).copied())
}

/// Which instances a resolution counts as inside the top modal layer, where `ModalRule::InDialogs` and `Blocked`
/// commands reach.
#[derive(Clone, Copy)]
enum Reach<'a> {
    /// A source with no component of its own (a menu, a shortcut): the instances around the key focus.
    Focus,
    /// A component: the instances around it, so a control inside a modal reaches the instances it sits in wherever
    /// the focus is.
    Chain(&'a CommandChain),
}

/// The vantage of the window under the modal layer `layer`, as it is once the layer closes (`vantage_under`).
fn beneath(ctx: &Context, layer: Id) -> Vantage {
    vantage_under(ctx, layer, focus::return_stop(ctx, layer))
}

/// The vantage of a component inside the menu `menu`, as it is once the menus close: from the stop focus then goes
/// to (`focus::out_of_menus`), under the chain's outermost menu. `None` when the walk finds no stop.
fn out_of_menu(ctx: &Context, menu: Id) -> Option<Vantage> {
    let out = focus::out_of_menus(ctx, Some(menu))?;
    let rec = focus::stop_rec(ctx, out.to)?;
    let scopes = focus::scope_chain(ctx, &rec);
    Some(vantage_under(ctx, out.root, Some((rec, scopes))))
}

/// The vantage of the window under the layer `layer` once it closes, with focus on `stop` (and its scope chain): the
/// modal layers below `layer` and in other viewports, and the stop, with the instances around it and the edits it
/// takes.
fn vantage_under(ctx: &Context, layer: Id, stop: Option<(focus::StopRec, Vec<Id>)>) -> Vantage {
    let viewport = pass::current_viewport(ctx);
    let (modal_open, root) = overlay::beneath(ctx, layer);
    let dialog_keys = keys::owner_at(ctx, stop.as_ref().map(|(rec, _)| rec), stop.as_ref().map_or(&[][..], |(_, scopes)| scopes), Some(layer))
        == KeyOwner::Class(OwnerClass::Dialog);
    let (edits, inactive) = stop.as_ref().map_or((EditSet::NONE, EditSet::NONE), |(rec, _)| (rec.stop.keys.edits, rec.stop.keys.inactive));
    let returns = stop.map(|(rec, _)| rec.stop.key().0);
    with(ctx, |board| {
        let chain: Vec<Id> = returns
            .and_then(|id| [board.chains.get(&viewport), board.before.get(&viewport)].into_iter().flatten().find(|(held, _)| *held == id))
            .map(|(_, chain)| chain.ids().collect())
            .unwrap_or_default();
        // Nothing to return to inside a modal: the modal's root takes the commands, as in `begin`.
        let chain = if chain.is_empty() && returns.is_none() { root.into_iter().collect() } else { chain };
        let inside_modal = Board::inside_modal_of(&chain, |id| Some(id) == root);
        Vantage { modal_open, dialog_keys, inside_modal, focused: vec![(viewport, chain)], edits: Some(edits), inactive: Some(inactive) }
    })
}

/// `command` as seen by `instance` (`None` for the app) from `from` (`None`: the key focus of the running pass), with
/// the gate applied, and the inputs it was made from; `reads` holds what the caller read to find the instance.
fn resolve_in(
    ctx: &Context,
    command: &CommandRef,
    instance: Option<Id>,
    reach: Reach<'_>,
    from: Option<&Vantage>,
    mut reads: Reads,
) -> (ResolvedCommand, ResolveKey) {
    let (resolved, key, first_unknown) = with(ctx, |board| {
        let first_unknown = board.def(command).is_none() && board.unknown.insert(command.id.clone());
        // A command no state named yet has a key too, so a state given later reaches the regions that read it.
        board.number(&command.id);
        let (mut resolved, state) = board.resolve(ctx, command, instance, reach, from.unwrap_or(&board.here), &mut reads);
        let gate = board.gate_answer(command);
        if gate.is_some_and(|(_, allowed)| !allowed) {
            resolved.enabled = false;
            resolved.refused = true;
        }
        let view = board.view_gens(pass::current_viewport(ctx));
        let key = ResolveKey {
            shared: board.gens.shared,
            keys: view.keys,
            focus: reads.focus.then_some((board.gens.focus, view.focus)),
            state: state.map(|state| (state, board.revisions.get(&state).copied().unwrap_or(0))),
            gate,
            edits: reads.edits.then_some(view.edits),
            vantage: None,
            number: board.numbers.get(command.id.as_str()).copied().unwrap_or(u32::MAX),
        };
        (resolved, key, first_unknown)
    });
    if first_unknown {
        log::warn!("the command {} is in no registered command set: it is an app command with its id as its label", command.id);
    }
    (resolved, key)
}

/// `command` for a surface with no component of its own: against the instance that `fire` would target.
pub(crate) fn resolve_global(ctx: &Context, command: &CommandRef) -> Arc<ResolvedCommand> {
    read(ctx, Reader::Global, command)
}

/// `command` for a surface with no component of its own that closes before its command runs, from the window under
/// the modal layer `layer` (`beneath`).
pub(crate) fn resolve_beneath(ctx: &Context, layer: Id, command: &CommandRef) -> Arc<ResolvedCommand> {
    read(ctx, Reader::Beneath(layer), command)
}

/// Step 1: the bound command of a component, resolved against its own instance. A component outside every instance
/// of the command's scope (a Navigator's Zoom In beside the canvas) reaches the instance a menu row of the command
/// reaches. A component inside a menu resolves it in its own chain for the instance (a panel menu's Float reaches the
/// group whose button opened it, focused or not), judged from the stop focus returns to once the menus close, which
/// gives the edits, the modal facts and the instance when the chain holds none (Cut in a text field's edit menu follows
/// the field's selection; an Edit menu row reaches the field that held focus); its activation fires that resolution.
pub(crate) fn resolve_bound(ctx: &Context, chain: &CommandChain, command: &CommandRef) -> Arc<ResolvedCommand> {
    let reader = match focus::running_menu(ctx) {
        Some(menu) => Reader::InMenu(menu, chain.clone()),
        None => Reader::Chain(chain.clone()),
    };
    read(ctx, reader, command)
}

/// `command` as `reader` is shown it, noted as its read: the kept resolution while its inputs are unchanged, else a new
/// one, kept.
fn read(ctx: &Context, reader: Reader, command: &CommandRef) -> Arc<ResolvedCommand> {
    // The resolved command carries the chord the keymap shows, kept or made now.
    crate::runtime::replay::note_input(ctx, crate::runtime::inputs::Input::Keymap);
    let viewport = pass::current_viewport(ctx);
    let vantage = vantage_of(ctx, &reader);
    let edits = focused_edits(ctx, viewport);
    let found = with(ctx, |board| board.lookup(viewport, &reader, command, vantage.as_ref(), edits));
    let (resolved, bind) = match found {
        Lookup::Fresh(resolved, bind) => (resolved, bind),
        found => {
            #[cfg(feature = "testing")]
            measure::count_resolve(ctx, if matches!(found, Lookup::Stale) { Resolve::Again } else { Resolve::New });
            let _ = found;
            let (resolved, key) = resolve_as(ctx, &reader, command, vantage);
            let (resolved, bind) = (Arc::new(resolved), key.bind());
            with(ctx, |board| board.keep(viewport, reader, Arc::clone(&resolved), key));
            (resolved, bind)
        }
    };
    note_read(ctx, bind);
    resolved
}

/// The vantage a layered reader is judged from: the window under its layer, or where focus goes once its menus close
/// (`None` when no stop is there). `None` for the other readers, which are judged from the board's own.
fn vantage_of(ctx: &Context, reader: &Reader) -> Option<Option<Vantage>> {
    match reader {
        Reader::Beneath(layer) => Some(Some(beneath(ctx, *layer))),
        Reader::InMenu(menu, _) => Some(out_of_menu(ctx, *menu)),
        Reader::Global | Reader::Chain(_) => None,
    }
}

/// `command` from the vantage of `reader` (`vantage`, from `vantage_of`), and the inputs it was made from. The
/// readers' first resolution and the one `begin` makes again both come from here.
fn resolve_as(ctx: &Context, reader: &Reader, command: &CommandRef, vantage: Option<Option<Vantage>>) -> (ResolvedCommand, ResolveKey) {
    let from = vantage.as_ref().and_then(Option::as_ref);
    let (resolved, mut key) = match reader {
        Reader::Global | Reader::Beneath(_) => resolve_from(ctx, command, from),
        Reader::Chain(chain) => resolve_chain(ctx, command, chain, None),
        Reader::InMenu(_, chain) => resolve_chain(ctx, command, chain, from),
    };
    key.vantage = vantage;
    (resolved, key)
}

/// The instance a source with no component of its own delivers `command` to from `from`, noting in `reads` that it
/// read the focus when it picked one.
fn target_read(ctx: &Context, board: &Board, command: &CommandRef, from: &Vantage, reads: &mut Reads) -> (bool, Option<Id>) {
    let (app, instance) = board.target_of(ctx, command, from);
    reads.focus |= !app;
    (app, instance)
}

/// `command` against the instances of `chain`, judged from `from` (`None`: the board's own vantage).
fn resolve_chain(ctx: &Context, command: &CommandRef, chain: &CommandChain, from: Option<&Vantage>) -> (ResolvedCommand, ResolveKey) {
    let mut reads = Reads::default();
    let instance = with(ctx, |board| {
        board.innermost(chain, board.scope_of(command)).or_else(|| target_read(ctx, board, command, from.unwrap_or(&board.here), &mut reads).1)
    });
    resolve_in(ctx, command, instance, Reach::Chain(chain), from, reads)
}

fn resolve_from(ctx: &Context, command: &CommandRef, from: Option<&Vantage>) -> (ResolvedCommand, ResolveKey) {
    let mut reads = Reads::default();
    let (_, instance) = with(ctx, |board| target_read(ctx, board, command, from.unwrap_or(&board.here), &mut reads));
    resolve_in(ctx, command, instance, Reach::Focus, from, reads)
}

/// `command` resolved once, for a key press or a fire: nothing keeps it.
fn resolve_once(ctx: &Context, command: &CommandRef, instance: Option<Id>, reach: Reach<'_>, from: Option<&Vantage>) -> ResolvedCommand {
    #[cfg(feature = "testing")]
    measure::count_resolve(ctx, Resolve::New);
    resolve_in(ctx, command, instance, reach, from, Reads::default()).0
}

/// fx.F23: resolves again what a reader of `viewport` was shown when an input of it changed since the viewport's last
/// wake, from that reader's own vantage under this pass's gate, states, modal facts and instances, and notes an edit of
/// the key each was read under when its resolution differs in any shown part (label, chord, enabled, checked, marked,
/// hint), so a settled memoized view that shows it runs; the entry keeps the new resolution. Readers of regions that did not
/// run keep their entries: a settled memoized view is the reader this exists for. It walks only the entries whose inputs
/// moved, through the table's indexes: every entry when a generation all of them read moved (the shared one, the
/// viewport's keys, the gate), else the readers of the focus, of the focused stop's edits and of each changed state,
/// and the readers under a layer or in a menu, whose vantage it reads again. A second pass of a frame compares against
/// what the first pass's runs were shown, so it wakes only the readers whose resolution the first pass changed (a modal
/// layer it opened or closed), once.
fn wake_shown(ctx: &Context, viewport: ViewportId) {
    let taken = with(ctx, |board| {
        let now =
            Woke { gens: board.gens, view: board.view_gens(viewport), state_seq: board.state_seq, gate: board.gate.as_ref().map(CommandGate::key) };
        let shown = board.shown.get(&viewport)?;
        let woke = shown.woke;
        if woke == Some(now) && shown.layered.is_empty() {
            return None;
        }
        let all = woke.is_none_or(|woke| woke.gens.shared != now.gens.shared || woke.view.keys != now.view.keys || woke.gate != now.gate);
        let mut walk: HashSet<u64> = if all { shown.entries.keys().copied().collect() } else { shown.layered.clone() };
        if let Some(woke) = woke.filter(|_| !all) {
            if (woke.gens.focus, woke.view.focus) != (now.gens.focus, now.view.focus) {
                walk.extend(&shown.by_focus);
            }
            if woke.view.edits != now.view.edits {
                walk.extend(&shown.by_edits);
            }
            for (_, state) in board.state_log.iter().filter(|(seq, _)| *seq > woke.state_seq) {
                walk.extend(shown.by_state.get(state).into_iter().flatten());
            }
        }
        Some((now, walk, board.shown.remove(&viewport)?))
    });
    let Some((now, walk, mut table)) = taken else { return };
    let mut changed = Vec::new();
    // A reader beneath a layer or inside a menu that is no longer shown is gone with it.
    table.drop_where(|entry| entry.reader.layer().is_some_and(|layer| !overlay::is_open(ctx, layer)));
    let mut vantages: HashMap<Id, Option<Option<Vantage>>> = HashMap::new();
    for id in walk {
        let Some(entry) = table.entries.get_mut(&id) else { continue };
        #[cfg(feature = "testing")]
        measure::count_wake_walk(ctx);
        let vantage = match entry.reader.layer() {
            Some(layer) => vantages.entry(layer).or_insert_with(|| vantage_of(ctx, &entry.reader)).clone(),
            None => None,
        };
        let fresh = with(ctx, |board| {
            let gate = board.gate_answer(&entry.kept.resolved.command);
            holds(&mut entry.kept.key, board.gens, board.view_gens(viewport), &board.revisions, gate, vantage.as_ref())
        });
        if fresh {
            continue;
        }
        #[cfg(feature = "testing")]
        measure::count_resolve(ctx, Resolve::Again);
        let (resolved, key) = resolve_as(ctx, &entry.reader, &entry.kept.resolved.command, vantage);
        let was = entry.kept.key.bind();
        let resolved = if resolved == *entry.kept.resolved {
            Arc::clone(&entry.kept.resolved)
        } else {
            changed.extend(was);
            changed.extend(key.bind().filter(|bind| Some(*bind) != was));
            Arc::new(resolved)
        };
        table.replace(id, Kept { resolved, key });
    }
    table.woke = Some(now);
    with(ctx, |board| {
        // Resolutions read nothing into the table while it is out of the board; keep any that arrived all the same.
        if let Some(added) = board.shown.remove(&viewport) {
            for (_, entry) in added.entries {
                let number = entry.kept.key.number;
                table.keep(entry.reader, number, entry.kept);
            }
        }
        if !table.entries.is_empty() {
            board.shown.insert(viewport, table);
        }
    });
    for key in changed {
        lazy::note_edit(ctx, key);
    }
}

/// Testing check, every 100th pass of a viewport: every kept resolution of `viewport` equals a full resolution from its
/// reader, and every kept gate answer equals the gate's answer asked again apart from the cache. A difference is an
/// input the keys miss.
#[cfg(feature = "testing")]
fn check_kept(ctx: &Context, viewport: ViewportId) {
    let Some(table) = with(ctx, |board| board.shown.remove(&viewport)) else { return };
    for entry in table.entries.values() {
        if entry.reader.layer().is_some_and(|layer| !overlay::is_open(ctx, layer)) {
            continue;
        }
        let command = &entry.kept.resolved.command;
        measure::count_resolve(ctx, Resolve::Checked);
        let (fresh, _) = resolve_as(ctx, &entry.reader, command, vantage_of(ctx, &entry.reader));
        assert!(
            fresh == *entry.kept.resolved,
            "the kept resolution of {} for a {} reader differs from a full resolve: kept {:?}, resolved {fresh:?}",
            command,
            entry.reader.kind(),
            entry.kept.resolved,
        );
        let (cached, gate) = with(ctx, |board| (board.gate_answers.get(command.key_text().as_ref()).copied(), board.gate.clone()));
        if let (Some(cached), Some(gate)) = (cached, gate) {
            assert!(cached == gate.allows(command), "the kept gate answer for {command} differs from the gate's");
        }
    }
    with(ctx, |board| {
        board.shown.insert(viewport, table);
    });
}

/// Step-1 resolution for a context in `chain`; `None` when the command's scope has no instance in the chain.
pub(crate) fn resolve_scoped(ctx: &Context, chain: &CommandChain, command: &CommandRef) -> Option<Arc<ResolvedCommand>> {
    let (found, app) = with(ctx, |board| {
        let scope = board.scope_of(command);
        (board.innermost(chain, scope).is_some(), board.to_app(scope))
    });
    if !app && !found {
        return None;
    }
    Some(resolve_bound(ctx, chain, command))
}

fn queue(ctx: &Context, fired: Fired) {
    #[cfg(feature = "testing")]
    with(ctx, |board| {
        if let Some(observed) = &mut board.observed {
            observed.push(fired.command.clone());
        }
    });
    // A command run from a menu or the palette whose key the focused component claims reaches it as that key, as the
    // key's press does (a button's is the button's command, even while a canvas gesture claims every key); an edit
    // command goes to the focused component that lists it; otherwise it goes to the instance that declared it or, with
    // none, stays an app command.
    if fired.phase != FirePhase::Release && matches!(fired.source, FireSource::Menu | FireSource::Palette) {
        let chords: Vec<Chord> =
            bindings(ctx, &fired.command).into_iter().filter(|binding| !binding.trigger.is_double()).map(|binding| binding.trigger.chord).collect();
        if keys::fire_claimed(ctx, &chords) {
            return;
        }
    }
    if fired.phase != FirePhase::Release
        && let Some(edit) = with(ctx, |board| board.registry.edit_of(&fired.command.id))
        && keys::queue_edit(ctx, edit)
    {
        return;
    }
    // An app that reads the clipboard through the board (`CommandInput::paste_reads_clipboard`) gets a Paste fired at
    // it from a menu, a button or the palette with the clipboard's content: the platform is asked in this pass, and
    // the paste it delivers fires the command (`deliver_requested_paste`).
    let reads_clipboard = fired.phase != FirePhase::Release
        && fired.target.is_none()
        && fired.pasted.is_none()
        && fired.source != FireSource::Key
        && with(ctx, |board| board.paste_reads_clipboard && board.registry.edit_of(&fired.command.id) == Some(EditCommand::Paste));
    if reads_clipboard {
        with(ctx, |board| {
            board.pending_pastes.insert(pass::current_viewport(ctx), fired);
        });
        crate::runtime::egui::clipboard::request_paste(ctx);
        return;
    }
    with(ctx, |board| if fired.target.is_some() { board.queue.push(fired) } else { board.out.push(fired) });
    // A scoped entry is taken next pass, so the root viewport runs one.
    repaint::request_of(ctx, ViewportId::ROOT);
}

/// An undo or redo a text part or editor had nothing to do for goes to the instance that declares the command or,
/// with none, to the app command of the same id (11.3), when it is enabled there as its key would be: the gate and the
/// modal rule hold, so an undo in a dialog's field never undoes the document under the dialog.
pub(crate) fn fall_through(ctx: &Context, command: craft_behavior::EditCommand) {
    let Some(command) = EditCommands::of(command) else { return };
    let command = CommandRef::from(command);
    // The focused component already declined the command: it answers for it no more.
    let from = with(ctx, |board| Vantage { edits: Some(EditSet::NONE), inactive: Some(EditSet::NONE), ..board.here.clone() });
    let (app, instance) = with(ctx, |board| board.target_of(ctx, &command, &from));
    if !app && instance.is_none() {
        return;
    }
    let resolved = resolve_once(ctx, &command, instance, Reach::Focus, Some(&from));
    if !resolved.enabled {
        return;
    }
    // An app-scope command a modal layer's instance declares is delivered to that instance.
    let target = if app { resolved.instance } else { instance };
    let fired = Fired { target, ..Fired::new(command, FireSource::Key, FirePhase::Press, pass::now(ctx)) };
    with(ctx, |board| if fired.target.is_some() { board.queue.push(fired) } else { board.out.push(fired) });
    repaint::request_of(ctx, ViewportId::ROOT);
}

/// Step 17: the command of a component that was activated and enabled.
pub(crate) fn fire_bound(ctx: &Context, command: &ResolvedCommand, mods: craft_keymap::Mods) {
    fire_resolved(ctx, command, FireSource::Pointer, mods);
}

/// Queues `command` for its instance with `source`; false when disabled.
pub(crate) fn fire_resolved(ctx: &Context, command: &ResolvedCommand, source: FireSource, mods: craft_keymap::Mods) -> bool {
    if !command.enabled || !crate::runtime::measure::effect(ctx, "board::fire_resolved") {
        return false;
    }
    queue(ctx, Fired { target: command.instance, ..Fired::new(command.command.clone(), source, FirePhase::Press, pass::now(ctx)) }.mods(mods));
    true
}

pub(crate) fn fire_from(ctx: &Context, command: &CommandRef, source: FireSource) -> bool {
    fire_in(ctx, command, source, None)
}

pub(crate) fn request_paste_at(ctx: &Context, at: egui::Pos2) {
    let command = CommandRef::from(EditCommands::Paste);
    let (app, instance) = with(ctx, |board| board.target_of(ctx, &command, &board.here));
    if !app && instance.is_none() {
        return;
    }
    let resolved = resolve_once(ctx, &command, instance, Reach::Focus, None);
    if !resolved.enabled {
        return;
    }
    let target = if app { resolved.instance } else { instance };
    let fired = Fired { target, paste_at: Some(at), ..Fired::new(command, FireSource::Pointer, FirePhase::Press, pass::now(ctx)) };
    with(ctx, |board| {
        board.pending_pastes.insert(pass::current_viewport(ctx), fired);
    });
    crate::runtime::egui::clipboard::request_paste(ctx);
}

pub(crate) fn has_pending_paste(ctx: &Context, viewport: ViewportId) -> bool {
    with(ctx, |board| board.pending_pastes.contains_key(&viewport))
}

pub(crate) fn deliver_requested_paste(ctx: &Context, viewport: ViewportId, content: super::ClipContent) {
    with(ctx, |board| {
        if let Some(mut fired) = board.pending_pastes.remove(&viewport) {
            fired.pasted = Some(content);
            if fired.target.is_some() {
                board.queue.push(fired);
            } else {
                board.out.push(fired);
            }
        }
    });
    repaint::request_of(ctx, ViewportId::ROOT);
}

/// `fire_from` for a surface that closes before its command runs, from the window under the modal layer `layer`
/// (`beneath`).
pub(crate) fn fire_beneath(ctx: &Context, layer: Id, command: &CommandRef, source: FireSource) -> bool {
    let from = beneath(ctx, layer);
    fire_in(ctx, command, source, Some(&from))
}

fn fire_in(ctx: &Context, command: &CommandRef, source: FireSource, from: Option<&Vantage>) -> bool {
    let (app, instance) = with(ctx, |board| board.target_of(ctx, command, from.unwrap_or(&board.here)));
    if !app && instance.is_none() {
        return false;
    }
    let resolved = resolve_once(ctx, command, instance, Reach::Focus, from);
    if !resolved.enabled {
        return false;
    }
    // An app-scope command a modal layer's instance declares is delivered to that instance.
    let target = if app { resolved.instance } else { instance };
    // A command fired from no component of the window (a native menu, the palette) closes the open popups, as a key's
    // command and a menu pick do, except the popup its instance is in and the popups it was opened from.
    let inside: Vec<Id> = with(ctx, |board| target.and_then(|target| board.instances.get(target)?.layer).into_iter().collect());
    overlay::close_popups_outside(ctx, &inside);
    queue(ctx, Fired { target, ..Fired::new(command.clone(), source, FirePhase::Press, pass::now(ctx)) });
    true
}

/// Publishes `state` of `command` for the innermost instance in `chain`. A state other than the one resolved so far in
/// the frame is an edit of it: a settled memoized view that read it runs, and so does the pass, when something read it
/// earlier in the pass.
///
/// A publish is a board effect, which a measuring run skips as it skips registration: the real run of the same content
/// publishes, and its edit check sees the reads the measuring runs made.
pub(crate) fn publish(ctx: &Context, chain: &CommandChain, command: &CommandRef, state: CommandState) {
    if !measure::effect(ctx, "commands::publish") {
        return;
    }
    replay::journal(
        ctx,
        || (chain.clone(), command.clone(), state.clone()),
        |ctx, (chain, command, state)| publish(ctx, chain, command, state.clone()),
    );
    let changed = with(ctx, |board| {
        let scope = board.scope_of(command);
        let declared = board.def(command).is_some_and(|def| def.modal == ModalRule::Declared);
        debug_assert!(
            !is_app(scope) || declared,
            "{command} is an app-scope command: its state comes from `FrameInput.commands`, unless the command is declared by an instance"
        );
        // A declared app-scope command is published by the innermost instance around the caller, inside whichever
        // modal layer it sits in.
        let instance = if is_app(scope) { chain.innermost_any() } else { board.innermost(chain, scope) };
        let instance = instance?;
        let number = board.number(&command.id);
        let key = (number, Some(instance));
        // The pass that publishes is the one that knows the layers around the caller.
        let in_modal = chain.ids().any(|id| board.is_modal_root(id));
        if in_modal && board.modal_published.insert(key) && !board.modal_published_last.contains(&key) {
            board.gens.shared += 1;
        }
        let changed = !same_state(board.this.get(&key).or_else(|| board.last.get(&key)), Some(&state));
        board.this.insert(key, state);
        if changed {
            board.bump_state(key);
        }
        changed.then_some((number, instance))
    });
    if let Some((number, instance)) = changed {
        let key = BindKey::Command(number, Some(instance));
        lazy::note_edit(ctx, key);
        if reads::earlier(ctx, key, instance).other {
            log::debug!(target: crate::runtime::egui::sizing::RERUN_LOG, "{instance:?} published a new state of {command}, read earlier in the pass");
            request_discard(ctx, Rerun::Edit);
        }
    }
}

/// The next command `from_ref` reads that is routed to `instance`. The instance of the app scope (an `AppShell`'s
/// region) takes the app commands of the frame that no earlier reader took; the rest stay for `FrameOutput.fired`.
pub(crate) fn take<S>(ctx: &Context, instance: Option<Id>, app: bool, from_ref: fn(&CommandRef) -> Option<S>) -> Option<(S, Fired)> {
    let instance = instance?;
    // Commands are for the real run: a copy would never run dry in the callers' `while let` loops.
    if crate::runtime::measure::active(ctx) {
        return None;
    }
    with(ctx, |board| {
        let fired = if app {
            let at = board.out.iter().position(|fired| from_ref(&fired.command).is_some())?;
            board.out.remove(at)
        } else {
            let at = board.queue.iter().position(|fired| fired.target == Some(instance) && from_ref(&fired.command).is_some())?;
            board.queue.remove(at)
        };
        board.delivered.push(fired.clone());
        Some((from_ref(&fired.command)?, fired))
    })
}

/// The next command of set `S` routed to the instance of its scope at `at` (a canvas that is one itself) or, when
/// `at` is no instance of it, to the innermost instance in `around`, the chain of the context `at` was shown in.
#[doc(hidden)]
pub fn take_around<S: CommandSet>(ctx: &Context, at: Id, around: &CommandChain) -> Option<(S, Fired)> {
    let scope = S::SCOPE.name();
    let instance = with(ctx, |board| board.instance_around(at, around, scope));
    take(ctx, instance, is_app(scope), S::from_ref)
}

/// The command of set `S` that `chord` runs at `at` (a canvas that is an instance of a scope itself) inside `around`:
/// the lookup for a key that automation holds, so it starts what the real key starts. The key is matched as the key
/// stage matches a press with `at` in focus (the viewport's layout, the keymap's scopes, panel fallthrough), on its
/// own since a held key completes no double press, and not while the focused component keeps the keys for typing;
/// the command goes where `take_around` takes it from and runs only
/// when it is enabled there, the gate included.
#[doc(hidden)]
pub fn bound_around<S: CommandSet>(ctx: &Context, at: Id, around: &CommandChain, chord: &Chord) -> Option<S> {
    let layout = keys::layout_of(ctx);
    let typing = focus::typing(ctx, pass::current_viewport(ctx));
    let (command, instance) = with(ctx, |board| {
        let mut chain: Vec<Id> = around.ids().collect();
        if !chain.contains(&at) {
            chain.push(at);
        }
        let hit = board.key_scopes(&chain)?.press(&layout, &chord_input(chord), typing, None)?;
        S::from_ref(&hit.command)?;
        Some((hit.command, board.instance_around(at, around, S::SCOPE.name())))
    })?;
    let resolved = resolve_once(ctx, &command, instance, Reach::Chain(around), None);
    if resolved.enabled { S::from_ref(&command) } else { None }
}

/// Whether the keymap binds `chord` to an enabled scoped command of the focused component's command chain, matched
/// as the key stage matches the press. A key the focused stop would activate with is left to that command.
pub(crate) fn binds_scoped(ctx: &Context, chord: &Chord) -> bool {
    let viewport = pass::current_viewport(ctx);
    let focused = focus::record(ctx, viewport).focused;
    let layout = keys::layout_of(ctx);
    let found = with(ctx, |board| {
        let chain = board.chains.get(&viewport).filter(|(id, _)| Some(*id) == focused).map(|(_, chain)| chain.clone())?;
        let ids: Vec<Id> = chain.ids().collect();
        let keys = board.key_scopes(&ids)?;
        let hit = keys.press(&layout, &chord_input(chord), false, None)?;
        // The lookup also ranks panels that share with the focused one and, with fallthrough, every other panel: only
        // a command of a scope around the focus counts.
        let command = std::iter::once(&hit.command).chain(&hit.cycle).find(|command| keys.scopes.contains(&board.scope_of(command)))?.clone();
        let instance = board.innermost(&chain, board.scope_of(&command));
        Some((command, instance, chain))
    });
    // Key routing runs before the command board refreshes its focus vantage for this pass. Check the actual focused
    // chain, so a focus that just entered a modal can reach its own commands without opening the modal to outsiders.
    found.is_some_and(|(command, instance, chain)| resolve_once(ctx, &command, instance, Reach::Chain(&chain), None).enabled)
}

/// Where a shortcut pressed with focus in a menu or on a menu bar matches, and where a hit gives the focus back. The
/// shortcut leaves the menus and the bar: it matches in the scopes of the stop focus returns to once every menu
/// closes, as with the menus closed. A menu bar that took the focus (F10, then Ctrl+S, or F10, Down, then Ctrl+S)
/// passes it on to the stop it took the focus from. The key-run command then closes the open menus as any does.
#[derive(Clone, Copy)]
enum Returning {
    /// The stop a menu bar took the focus from, which `focus::give_back_bar` gives it back to.
    Bar(Id),
    /// The stop focus returns to once every menu around it closes.
    Stop(focus::StopKey),
}

impl Returning {
    /// Where a shortcut pressed with `focused` holding the focus goes: from a stop in a menu, where focus goes once
    /// the menus close (`focus::out_of_menus`); from the bar, on to the stop it took the focus from. `None` when the
    /// focus is in neither, or a menu opened with nothing focused.
    fn of(ctx: &Context, focused: Id) -> Option<Self> {
        focus::returning(ctx, focused).map(|(to, bar)| if bar { Self::Bar(to.0) } else { Self::Stop(to) })
    }

    /// The component whose command chain the shortcut matches in.
    fn id(self) -> Id {
        match self {
            Self::Bar(id) | Self::Stop((id, _)) => id,
        }
    }

    /// A hit gives the focus back, in this pass.
    fn give_back(self, ctx: &Context) {
        match self {
            Self::Bar(_) => {
                focus::give_back_bar(ctx);
            }
            Self::Stop((id, part)) => {
                focus::request(ctx, part.map_or(focus::FocusTarget::Id(id), |part| focus::FocusTarget::Part(id, part)));
            }
        }
    }
}

/// The release of the momentary or tool command `held`, fired when its key goes up at `place` among the pass's events
/// or the window loses focus (the key's release then goes to another window).
fn release_fired(held: Held, mods: craft_keymap::Mods, time: f64, place: Option<usize>) -> Fired {
    Fired {
        command: held.command,
        target: held.target,
        source: FireSource::Key,
        phase: FirePhase::Release,
        cycle: false,
        alt_variant: false,
        mods,
        time,
        place,
        pasted: None,
        paste_at: None,
    }
}

/// H7: the key stage's shortcut events, matched against the keymap in the scopes around the focused component,
/// through the gate, to their commands. A key with a hold binding waits (`HoldWait`) for its release, the next event
/// or its deadline.
pub(crate) fn begin(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    let focused = focus::record(ctx, viewport).focused;
    let shortcuts = keys::take_shortcuts(ctx);
    let now = pass::now(ctx);
    let held_open = sampler::held_canvas(ctx);
    // A shortcut pressed in a menu or on a menu bar matches where the focus returns to once the menus close.
    let returning = focused.filter(|_| !shortcuts.keys.is_empty()).and_then(|focused| Returning::of(ctx, focused));
    let shell = frame::shell_root(ctx, viewport);
    let edits = focused_edits(ctx, viewport);
    let taken = with(ctx, |board| {
        if board.held_open != held_open {
            board.held_open = held_open;
            board.gens.shared += 1;
        }
        let returned = returning
            .map(Returning::id)
            .and_then(|held| [board.before.get(&viewport), board.chains.get(&viewport)].into_iter().flatten().find(|(id, _)| *id == held))
            .map(|(_, chain)| chain.ids().collect());
        let chain: Vec<Id> = returned.unwrap_or_else(|| {
            board.chains.get(&viewport).filter(|(id, _)| Some(*id) == focused).map(|(_, chain)| chain.ids().collect()).unwrap_or_default()
        });
        // Nothing focused inside a modal: the modal's root takes the commands, as a plain Enter goes to its default button.
        let chain = match (chain.is_empty() && focused.is_none(), board.modal_roots.get(&viewport)) {
            (true, Some(root)) => vec![*root],
            _ => chain,
        };
        let view = board.views.entry(viewport).or_default();
        if !chain.is_empty() && view.last_focused != chain {
            view.last_focused.clone_from(&chain);
            view.gens.focus += 1;
        }
        // Nothing focused outside a modal: the window's shell root takes the commands, so a window command's key comes
        // before an app command's, and the page region's after it. Nothing holds focus there, so the chain last focused
        // stays.
        let chain = if chain.is_empty() && focused.is_none() {
            let page = board.page_roots.get(&viewport).copied();
            shell.into_iter().chain(page).collect()
        } else {
            chain
        };
        match board.here.focused.iter_mut().find(|(v, _)| *v == viewport) {
            Some(entry) if entry.1 == chain => {}
            Some(entry) => {
                entry.1.clone_from(&chain);
                board.gens.focus += 1;
            }
            None => {
                board.here.focused.push((viewport, chain.clone()));
                board.gens.focus += 1;
            }
        }
        // The viewport's own facts move its own generations; `here` holds the running viewport's for the pass.
        let inside_modal = Board::inside_modal_of(&chain, |id| board.is_modal_root(id));
        let view = board.views.entry(viewport).or_default();
        if view.dialog_keys != shortcuts.dialog {
            view.dialog_keys = shortcuts.dialog;
            view.gens.keys += 1;
        }
        if view.inside_modal != inside_modal {
            view.inside_modal = inside_modal;
            view.gens.focus += 1;
        }
        if view.edits != edits {
            view.edits = edits;
            view.gens.edits += 1;
        }
        board.here.dialog_keys = view.dialog_keys;
        board.here.inside_modal.clone_from(&view.inside_modal);
        let waiting = board.hold_wait.as_ref().is_some_and(|wait| wait.viewport == viewport);
        if shortcuts.keys.is_empty() && shortcuts.mods.is_empty() && !shortcuts.focus_lost && !waiting {
            return None;
        }
        let mut keys = board.key_scopes(&chain)?;
        // The held-open canvas's scope is around the focus for matching: its commands fire from the dialog.
        let held_scope = held_open.and_then(|id| board.instances.get(id)).map(|i| i.scope.name()).filter(|scope| !is_app(scope));
        if let Some(scope) = held_scope
            && !keys.scopes.contains(&scope)
        {
            keys.scopes.push(scope);
        }
        Some((keys, std::mem::take(&mut board.matcher)))
    });
    // With this pass's vantage set, what the readers were shown is resolved again before any of them runs.
    wake_shown(ctx, viewport);
    #[cfg(feature = "testing")]
    if pass::pass_nr(ctx, viewport).is_multiple_of(100) {
        check_kept(ctx, viewport);
    }
    let Some((keys, mut matcher)) = taken else { return };
    let modal_at_start = overlay::any_open(ctx, overlay::OpenFilter::Modal);
    let key_fired = |hit: Hit, target: Option<Id>, time: f64, place: Option<usize>| Fired {
        command: hit.command,
        target,
        source: FireSource::Key,
        phase: FirePhase::Press,
        cycle: hit.cycle.len() > 1,
        alt_variant: hit.alt_variant,
        mods: hit.trigger.chord.mods,
        time,
        place,
        pasted: None,
        paste_at: None,
    };
    let (delay, mut wait) = with(ctx, |board| {
        let wait = if board.hold_wait.as_ref().is_some_and(|wait| wait.viewport == viewport) { board.hold_wait.take() } else { None };
        (board.hold_delay, wait)
    });
    // A modal layer or a text field that has the keys ends the wait without firing: the key is no longer the shortcut's.
    // PLAN: a tool claim or an IME composition on the key also ends it; the claim stage (kb.C11c, shortcuts-w2) gives the board no signal for either yet.
    if modal_at_start || shortcuts.typing {
        wait = None;
    }
    // The fired press of `hit`, or `None` when its command is disabled now or blocked by a modal layer an earlier hit of
    // the pass opened (kb.C9). A momentary or tool command is kept with its key, whose release fires it again.
    let press = |hit: Hit, key: Option<craft_keymap::Key>, time: f64, place: Option<usize>| -> Option<Fired> {
        let (blocked, holds_key) = with(ctx, |board| {
            board.def(&hit.command).map_or((false, false), |def| {
                (def.modal == ModalRule::Blocked, matches!(def.kind, CommandKind::Momentary | CommandKind::Tool { .. }))
            })
        });
        if blocked && !modal_at_start && overlay::any_open(ctx, overlay::OpenFilter::Modal) {
            return None;
        }
        let (app, instance) = with(ctx, |board| board.target_of(ctx, &hit.command, &board.here));
        let resolved = resolve_once(ctx, &hit.command, instance, Reach::Focus, None);
        if !resolved.enabled {
            decline(ctx, hit, app, &resolved, time);
            return None;
        }
        // An app-scope command a modal layer's instance declares is delivered to that instance.
        let target = if app { resolved.instance } else { instance };
        let pasted = with(ctx, |board| {
            if let Some(key) = key.filter(|_| holds_key) {
                board.held.retain(|held| held.key != key);
                board.held.push(Held { key, command: hit.command.clone(), target });
            }
            board.registry.edit_of(&hit.command.id) == Some(EditCommand::Paste)
        });
        let mut fired = key_fired(hit, target, time, place);
        fired.pasted = shortcuts.pasted.clone().filter(|_| pasted);
        Some(fired)
    };
    // Settles a wait by the time of the event that ends it: the hold when the key was down for the delay, else the tap.
    let settle = |wait: Option<HoldWait>, time: f64, place: Option<usize>| -> Option<Fired> {
        let wait = wait?;
        let hit = if time - wait.since >= wait.delay { Some(wait.hold) } else { wait.tap }?;
        press(hit, Some(wait.key), time, place)
    };
    // A binding whose command is disabled where the press lands (a window command with no floating window, a canvas's
    // view command it does not offer) is passed over: the press runs the next binding of its chord, in an outer scope,
    // or of its next chord candidate, as if the disabled one were unbound.
    let enabled = |command: &CommandRef| {
        let (_, instance) = with(ctx, |board| board.target_of(ctx, command, &board.here));
        resolve_in(ctx, command, instance, Reach::Focus, None, Reads::default()).0.enabled
    };
    // Whether a text component has the keys for `shortcut`'s press; one that lets command keys through leaves their
    // presses to the app's commands.
    let typing_at = |shortcut: &keys::Shortcut| {
        let command_key = shortcuts.command_keys
            && shortcuts
                .layout
                .candidates(&shortcut.input)
                .first()
                .is_some_and(|chord| TypingRule::CommandKeysOnly.fires(chord, keys.keymap.platform()));
        shortcuts.typing && !shortcut.canvas && !command_key
    };
    let (mut fired, mut dimmed) = (Vec::new(), Vec::new());
    let mut changes = shortcuts.mods.iter().map(|(index, _)| *index).peekable();
    for (index, shortcut) in shortcuts.keys.iter().enumerate() {
        // A change of Cmd, Ctrl or Alt before this key settles the wait first.
        while changes.next_if(|change| *change <= index).is_some() {
            fired.extend(settle(wait.take(), now, None));
        }
        // When the key event arrived: a frame may run it later, and a double press and a hold read the key's own times.
        let time = shortcut.at.unwrap_or(now);
        let key = shortcut.input.physical.or(shortcut.input.typed);
        match shortcut.phase {
            FirePhase::Release => {
                if wait.as_ref().is_some_and(|wait| Some(wait.key) == key) {
                    fired.extend(settle(wait.take(), time, Some(shortcut.place)));
                }
                let held = with(ctx, |board| key.and_then(|key| board.held.iter().position(|held| held.key == key)).map(|at| board.held.remove(at)));
                fired.extend(held.map(|held| release_fired(held, shortcut.input.mods, time, Some(shortcut.place))));
            }
            FirePhase::Repeat => {
                // A held key's repeat (`RootEnv::shortcuts_repeat`) runs its chord's command again, on its own: it
                // completes no double press and settles no wait, and the key a momentary or tool command holds stays
                // held by its press. The key of a wait or of a hold binding repeats nothing.
                let typing = typing_at(shortcut);
                if wait.as_ref().is_some_and(|wait| Some(wait.key) == key) || keys.hold(&shortcuts.layout, &shortcut.input, typing).is_some() {
                    continue;
                }
                let tap = keys.press(&shortcuts.layout, &shortcut.input, typing, Some((&mut matcher, time, true, &enabled)));
                let repeated = tap.and_then(|tap| press(tap, None, time, Some(shortcut.place)));
                fired.extend(repeated.map(|fired| Fired { phase: FirePhase::Repeat, ..fired }));
            }
            FirePhase::Press => {
                // Another key's press settles the wait first: keys act in the order they went down.
                fired.extend(settle(wait.take(), time, Some(shortcut.place)));
                let typing = typing_at(shortcut);
                let hold = key
                    .filter(|_| !typing && !modal_at_start)
                    .and_then(|key| keys.hold(&shortcuts.layout, &shortcut.input, typing).map(|hold| (key, hold)));
                let tap = keys.press(&shortcuts.layout, &shortcut.input, typing, Some((&mut matcher, time, false, &enabled)));
                match (hold, tap) {
                    (Some((key, hold)), tap) => wait = Some(HoldWait { key, hold, since: time, delay, tap, viewport }),
                    (None, Some(tap)) => fired.extend(press(tap, key, time, Some(shortcut.place))),
                    // A chord that binds only disabled commands runs nothing; the app hears of the first one, to say
                    // why it did not run as a menu row's hint does, and may receive it as declined.
                    (None, None) => {
                        if let Some(hit) = keys.press(&shortcuts.layout, &shortcut.input, typing, None) {
                            let (app, instance) = with(ctx, |board| board.target_of(ctx, &hit.command, &board.here));
                            let resolved = resolve_once(ctx, &hit.command, instance, Reach::Focus, None);
                            decline(ctx, hit.clone(), app, &resolved, time);
                            dimmed.push(key_fired(hit, instance, time, Some(shortcut.place)));
                        }
                    }
                }
            }
        }
    }
    for _ in changes {
        fired.extend(settle(wait.take(), now, None));
    }
    // The key's release goes to another window: a momentary command ends and a wait drops.
    if shortcuts.focus_lost {
        wait = None;
        let held = with(ctx, |board| std::mem::take(&mut board.held));
        fired.extend(held.into_iter().map(|held| release_fired(held, craft_keymap::Mods::NONE, now, None)));
    }
    // The deadline is a time event: only the first pass of a frame, which has elapsed time, fires it.
    if let Some(due) = wait.as_ref().map(|wait| wait.since + wait.delay) {
        if now < due {
            frame::request_repaint_after(ctx, due - now);
        } else if frame::running(ctx).is_some_and(|pass| pass.elapsed > 0.0) {
            fired.extend(wait.take().and_then(|wait| press(wait.hold, Some(wait.key), now, None)));
        }
    }
    with(ctx, |board| {
        board.matcher = matcher;
        board.dimmed.append(&mut dimmed);
        if wait.is_some() {
            board.hold_wait = wait;
        }
    });
    let pressed = fired.iter().any(|fired| fired.phase == FirePhase::Press);
    if pressed && let Some(returning) = returning {
        returning.give_back(ctx);
    }
    // A command a key runs closes the open popups (menus, dropdowns, lists), as a menu pick does, except the popup
    // whose content declares the command's scope (a tour card's own arrows) and the popups it was opened from.
    if pressed {
        let inside: Vec<Id> = with(ctx, |board| {
            fired
                .iter()
                .filter(|fired| fired.phase == FirePhase::Press)
                .filter_map(|fired| fired.target)
                .filter_map(|target| board.instances.get(target)?.layer)
                .collect()
        });
        overlay::close_popups_outside(ctx, &inside);
    }
    for fired in fired {
        queue(ctx, fired);
    }
}

/// The end hook: instances that stopped registering are gone, and so are the commands queued for them and the readers
/// inside them.
pub(crate) fn end(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    with(ctx, |board| {
        let live = pass::live_viewports(ctx);
        let alive: Vec<bool> = board.instances.iter().map(|i| board.live(ctx, i) && live.contains(&i.viewport)).collect();
        // An instance of this viewport this pass did not register is dead from the viewport's next pass: resolutions
        // see it.
        let dies = board.instances.iter().zip(&alive).any(|(i, alive)| *alive && i.viewport == viewport && i.stamp < pass::pass_nr(ctx, viewport));
        let gone: HashSet<Id> = board.instances.iter().zip(&alive).filter(|(_, alive)| !**alive).map(|(i, _)| i.id).collect();
        // The revisions of a removed instance's states go in the same step as a shared move, which makes every entry
        // that holds one stale first.
        if !gone.is_empty() {
            board.revisions.retain(|(_, instance), _| instance.is_none_or(|id| !gone.contains(&id)));
        }
        if dies || !gone.is_empty() {
            board.gens.shared += 1;
        }
        let mut keep = alive.iter();
        board.instances.retain(|_| keep.next().copied().unwrap_or(false));
        let instances = &board.instances;
        let ids = |id: Id| instances.contains(id);
        board.queue.retain(|fired| fired.target.is_some_and(ids));
        board.held.retain(|held| held.target.is_none_or(ids));
        if board.hold_wait.as_ref().is_some_and(|wait| !live.contains(&wait.viewport)) {
            board.hold_wait = None;
        }
        board.chains.retain(|viewport, _| live.contains(viewport));
        board.before.retain(|viewport, _| live.contains(viewport));
        board.views.retain(|viewport, _| live.contains(viewport));
        board.here.focused.retain(|(viewport, _)| live.contains(viewport));
        board.shown.retain(|viewport, table| {
            table.drop_where(|entry| match &entry.reader {
                Reader::Chain(chain) | Reader::InMenu(_, chain) => chain.innermost_any().is_some_and(|id| !ids(id)),
                _ => false,
            });
            live.contains(viewport) && !table.entries.is_empty()
        });
        // The state log keeps the changes no kept table has woken past.
        let oldest = board.shown.values().map(|table| table.woke.map_or(0, |woke| woke.state_seq)).min();
        match oldest {
            Some(oldest) => board.state_log.retain(|(seq, _)| *seq > oldest),
            None => board.state_log.clear(),
        }
    });
}
