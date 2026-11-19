//! The board: state by `(command, instance)`, the instances of each scope, the queue, resolution, firing and the
//! dispatcher that turns the key stage's shortcut events into fired commands (spec 11.3).

use super::{
    AppCatalog, CommandChain, CommandGate, CommandInput, CommandScope, CommandSet, CommandState, EditCommands, FirePhase, FireSource, Fired,
    ResolvedCommand, WindowShortcut, registry,
};
use crate::runtime::egui::sizing::{Rerun, request_discard};
use crate::runtime::egui::{pass, repaint};
use crate::runtime::focus::EditSet;
use crate::runtime::pass_state;
use crate::runtime::reads::{self, BindKey};
use crate::runtime::{focus, frame, keys, lazy, measure, overlay, replay, sampler};
use craft_behavior::{EditCommand, KeyOwner, OwnerClass, pick_instance, resolve_flags};
use craft_keymap::{
    Active, Chord, CommandDef, CommandKind, CommandRef, Hit, KeyInput, Keymap, Layout, Matcher, ModalRule, Overrides, Platform, Press, Scopes,
    Trigger,
};
use egui::{Context, Id, ViewportId};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

type StateKey = (u32, Option<Id>);

/// How a command with `ModalRule::Declared` reaches into the top modal layer.
#[derive(Clone, Copy)]
enum Declaration {
    /// An instance publishes the command's state.
    Instance(Id),
    /// The focused component takes the command as an edit; `inactive` when the edit does nothing now
    /// (`KeyUse::inactive`), so the command resolves disabled.
    Focused { inactive: bool },
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
    /// The chords bound to the edit commands, which the input hook removes from the raw input when the focused
    /// component lists the command.
    edit_chords: Arc<[(EditCommand, Chord)]>,
    /// The chords bound to the window's keys, which the input hook takes before any component.
    window_shortcuts: Arc<[(WindowShortcut, Chord)]>,
}

/// A momentary command whose key is down: its release fires the command again.
struct Held {
    key: craft_keymap::Key,
    command: CommandRef,
    target: Option<Id>,
}

#[derive(Default)]
pub(crate) struct Board {
    registry: Arc<registry::Registry>,
    /// The commands the app defines at run time, from this frame's input.
    app: Option<AppCatalog>,
    keymap: Option<KeymapCache>,
    gate: Option<CommandGate>,
    /// The gate's answers in this frame, by command key (`CommandRef::key`). The gate answers from what the app knew
    /// before the frame, so the board asks it once per command per frame, whichever readers and passes resolve it.
    gate_answers: HashMap<String, bool>,
    /// Command ids by number, so a state lookup allocates nothing.
    numbers: HashMap<String, u32>,
    this: HashMap<StateKey, CommandState>,
    last: HashMap<StateKey, CommandState>,
    /// What the frame's input changed, under the root frame it arrived for (`set_input`).
    changed: (u64, InputChanges),
    /// Scoped commands waiting for their instance.
    queue: Vec<Fired>,
    /// App-scope commands for the next `FrameOutput`.
    out: Vec<Fired>,
    /// The commands instances took since the last `FrameOutput`, in order.
    delivered: Vec<Fired>,
    instances: Vec<Instance>,
    /// The focused component of each viewport's last step 12 and the instances around it.
    chains: HashMap<ViewportId, (Id, CommandChain)>,
    /// The focused component and its chain before the one in `chains`, per viewport; while the focus is in the menus
    /// or on a menu bar, the stop it returns to once they close, whose scopes a shortcut pressed there matches in.
    before: HashMap<ViewportId, (Id, CommandChain)>,
    /// Where the commands of the running pass are judged from: the key focus and the modal layers.
    here: Vantage,
    /// The chain of the last pass in which any component held key focus.
    last_focused: Vec<Id>,
    matcher: Matcher,
    held: Vec<Held>,
    /// The top modal layer's root container of each viewport that has one.
    modal_roots: Vec<(ViewportId, Id)>,
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
    /// What each reader was shown, by the state key it read and its viewport (`note_read`).
    shown: ShownMap,
    /// The scope each child scope is declared inside: an instance of the child is also an instance of the parent.
    parents: HashMap<&'static str, &'static str>,
}

/// What `set_input` changed for the readers of command states, which each pass of the frame notes before any component
/// runs (`begin`).
#[derive(Clone, Default)]
struct InputChanges {
    /// The keymap was rebuilt: any chord shown may differ.
    keymap: bool,
}

/// Whether a resolution finds the same state under `a` and `b`: no state is the default one.
fn same_state(a: Option<&CommandState>, b: Option<&CommandState>) -> bool {
    a.cloned().unwrap_or_default() == b.cloned().unwrap_or_default()
}

/// Who read a command's resolution: the vantage `begin` resolves it from again.
#[derive(Clone, PartialEq, Eq)]
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

/// What a reader was shown the last time it resolved a command.
struct Shown {
    reader: Reader,
    resolved: ResolvedCommand,
}

/// The readers of each state key in each viewport.
type ShownMap = HashMap<(StateKey, ViewportId), Vec<Shown>>;

/// Keeps `resolved` as what `reader` of `viewport` was shown under `key`, in place of what it was shown before.
fn keep_shown(shown: &mut ShownMap, key: StateKey, viewport: ViewportId, reader: Reader, resolved: &ResolvedCommand) {
    let readers = shown.entry((key, viewport)).or_default();
    match readers.iter_mut().find(|entry| entry.reader == reader) {
        Some(entry) => entry.resolved.clone_from(resolved),
        None => readers.push(Shown { reader, resolved: resolved.clone() }),
    }
}

/// Notes, in the read record, that the running code showed the state under `key`, and keeps what `reader` was shown: a
/// settled region that showed it runs when `begin` resolves the command from the same reader to something else (a
/// state, the gate, a modal layer or an instance changed), and a change published after it in the pass runs the pass
/// again.
fn note_read(ctx: &Context, key: Option<StateKey>, reader: Reader, resolved: &ResolvedCommand) {
    let Some((number, instance)) = key else { return };
    reads::note(ctx, BindKey::Command(number, instance), Id::NULL, measure::active(ctx), &[]);
    let viewport = pass::current_viewport(ctx);
    with(ctx, |board| keep_shown(&mut board.shown, (number, instance), viewport, reader, resolved));
}

/// Where a resolution judges a command from: the modal facts and the focus. The board's own is the key focus of the
/// running pass; a surface that closes before its command runs (the palette) judges from the window under it.
#[derive(Clone, Default)]
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

impl Board {
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
        let own = self.instances.iter().any(|i| i.id == at && self.within(i.scope.name(), scope));
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
        self.instances.iter().any(|i| i.id == id && self.live(ctx, i))
    }

    /// The definition of the command `id`: the linked sets', else the frame's app catalog's.
    fn def(&self, id: &str) -> Option<&CommandDef> {
        self.registry.get(id).or_else(|| self.app.as_ref()?.get(id))
    }

    /// The scope name of `command`; commands no definition names belong to the application.
    fn scope_of(&self, command: &CommandRef) -> &str {
        self.def(&command.id).map_or(CommandScope::APP.name(), |def| def.scope.as_str())
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
        pick_instance(&self.live_instances(ctx, scope), &focused, &self.last_focused)
    }

    /// The non-app scopes around the focus, innermost first. An instance of a child scope also brings the scopes it
    /// is declared inside, after its own.
    fn scopes_around(&self, chain: &[Id]) -> Vec<&'static str> {
        let mut scopes: Vec<&'static str> = Vec::new();
        for instance in chain.iter().rev().filter_map(|id| self.instances.iter().find(|i| i.id == *id)) {
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
        self.modal_roots.iter().any(|(_, root)| *root == id)
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
    fn declaring(&self, ctx: &Context, command: &CommandRef, from: &Vantage) -> Option<Declaration> {
        let dialog_open = from.modal_open || from.dialog_keys;
        let number = *self.numbers.get(command.id.as_str())?;
        let published = self.modal_published.iter().chain(&self.modal_published_last).find(|(published, _)| *published == number);
        if let Some(id) = published.and_then(|(_, id)| *id).filter(|_| dialog_open) {
            return Some(Declaration::Instance(id));
        }
        let edit = self.registry.edit_of(&command.id)?;
        let keys = from
            .edits
            .map(|edits| (edits, from.inactive.unwrap_or(EditSet::NONE)))
            .or_else(|| focus::record(ctx, pass::current_viewport(ctx)).keys.map(|keys| (keys.edits, keys.inactive)));
        let in_reach = !dialog_open || !from.inside_modal.is_empty();
        let (edits, inactive) = keys.filter(|_| in_reach)?;
        edits.contains(edit).then_some(Declaration::Focused { inactive: inactive.contains(edit) })
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
    ) -> (ResolvedCommand, Option<StateKey>) {
        let def = self.def(&command.id);
        let app = self.to_app(self.scope_of(command));
        let instance = if app { None } else { instance };
        let dialog_open = from.modal_open || from.dialog_keys;
        // A command no set registers is never blocked.
        let rule = def.map(|def| def.modal);
        // The instance inside the top modal that declares the command, which then answers for it.
        let declaring = (rule == Some(ModalRule::Declared)).then(|| self.declaring(ctx, command, from)).flatten();
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
        let this = key.and_then(|key| self.this.get(&key));
        let last = key.and_then(|key| self.last.get(&key));
        let state = this.or(last);
        let flags = resolve_flags(this.map(CommandState::flags), last.map(CommandState::flags));
        let reached = declaring.is_some()
            || instance.is_some_and(|id| match reach {
                Reach::Focus => from.inside_modal.contains(&id),
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
        let chord = self.keymap.as_ref().and_then(|cache| cache.keymap.shown_chord(command));
        let resolved = ResolvedCommand {
            label: state.and_then(|s| s.title_ref().cloned()).unwrap_or_else(|| Arc::from(def.map_or(command.id.as_str(), |def| def.label.as_str()))),
            icon: def.and_then(|def| def.icon.clone()),
            chord,
            enabled: flags.enabled && modal && live && !matches!(declaring, Some(Declaration::Focused { inactive: true })),
            checked: flags.checked,
            marked: state.is_some_and(CommandState::is_marked),
            hint: state.and_then(|s| s.hint_ref().cloned()),
            kind: def.map_or(CommandKind::Command, |def| def.kind.clone()),
            command: command.clone(),
            instance,
        };
        (resolved, key)
    }
}

fn is_app(scope: &str) -> bool {
    scope == CommandScope::APP.name()
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
    /// What a press of `input` runs, read through `layout`, with `typing` when a text field has the keys. With
    /// `matcher` and the press's time the press counts toward a double press; without, it is matched on its own. A
    /// held scope's keys stay with the text while typing; a typing scope's keys are the text component's own.
    fn press(&self, layout: &Layout, input: &KeyInput, typing: bool, matcher: Option<(&mut Matcher, f64)>) -> Option<Hit> {
        let (panel, outer) = self.scopes.split_first().map_or((None, &[][..]), |(panel, outer)| (Some(*panel), outer));
        let held: &[&'static str] = if typing { &[] } else { &self.held };
        let contexts: Vec<&str> = self.typing.iter().chain(held).copied().collect();
        let active = Active { panel, contexts: &contexts, typing };
        match matcher {
            Some((matcher, now)) => matcher.press(layout, &self.keymap, &Press { input, repeat: false, now, active: &active, outer }),
            None => Matcher::single(layout, &self.keymap, input, &active, outer),
        }
    }
}

/// A press of `chord` known only by its key, as injected key events carry it: the typed key at its own position.
fn chord_input(chord: &Chord) -> KeyInput {
    KeyInput::new(chord.mods).with_typed(chord.key).with_physical(chord.key)
}

/// Loads the linked command sets; the stories' sets only with `stories`.
pub(crate) fn install(ctx: &Context, stories: bool) -> Result<(), registry::CommandDefError> {
    let registry = registry::Registry::shared(stories)?;
    with(ctx, |board| board.registry = registry);
    Ok(())
}

/// A keymap with the chords it binds to the edit commands and to the window's keys.
#[derive(Clone)]
struct BuiltKeymap {
    keymap: Arc<Keymap>,
    edit_chords: Arc<[(EditCommand, Chord)]>,
    window_shortcuts: Arc<[(WindowShortcut, Chord)]>,
}

/// The keymap of `defs` on `platform` under `overrides`.
fn keymap_of(defs: Vec<CommandDef>, scopes: Scopes, platform: Platform, overrides: Option<&Arc<Overrides>>) -> Keymap {
    let mut keymap = Keymap::new(defs, scopes, platform);
    if let Some(overrides) = overrides {
        keymap.set_overrides((**overrides).clone());
    }
    keymap
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
/// A window shortcut's default chord that another command also binds is that command's: the window shortcut loses it, so the
/// app's binding beats the framework's default. A window shortcut the overrides name keeps what they give it.
fn build_keymap(registry: &registry::Registry, app: Option<&AppCatalog>, platform: Platform, overrides: Option<&Arc<Overrides>>) -> BuiltKeymap {
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
    let mut keymap = keymap_of(joined(defs.as_slice()), scopes.clone(), platform, overrides);
    let overridden = |def: &CommandDef| overrides.is_some_and(|overrides| overrides.get(&def.command.id).is_some());
    let yielded: Vec<(CommandRef, Trigger)> = defs
        .iter()
        .filter(|def| WindowShortcut::of(&def.command).is_some() && !overridden(def))
        .flat_map(|def| keymap.conflicts_with(&def.command))
        .filter(|conflict| WindowShortcut::of(&conflict.b.0).is_none())
        .map(|conflict| (conflict.a.0, conflict.chord))
        .collect();
    if !yielded.is_empty() {
        for def in &mut defs {
            def.defaults
                .retain(|binding| !yielded.iter().any(|(command, chord)| *command == def.command && binding.trigger.effective(platform) == *chord));
        }
        keymap = keymap_of(joined(defs.as_slice()), scopes, platform, overrides);
    }
    let window_shortcuts = registry
        .defs()
        .iter()
        .filter_map(|def| WindowShortcut::of(&def.command).map(|key| (key, &def.command)))
        .flat_map(|(key, command)| {
            keymap.bindings(command).into_iter().filter(|binding| !binding.trigger.double).map(move |binding| (key, binding.trigger.chord))
        })
        .collect();
    let edit_chords = registry
        .defs()
        .iter()
        .filter_map(|def| registry.edit_of(&def.command.id).map(|edit| (edit, &def.command)))
        // Delete's keys (Delete, Backspace) stay a focused text part's own deletion, which also deletes one character.
        .filter(|(edit, _)| *edit != EditCommand::Delete)
        .flat_map(|(edit, command)| {
            keymap.bindings(command).into_iter().filter(|binding| !binding.trigger.double).map(move |binding| (edit, binding.trigger.chord))
        })
        .collect();
    BuiltKeymap { keymap: Arc::new(keymap), edit_chords, window_shortcuts }
}

/// The keymap of each shared registry (with or without the stories' sets) on each platform with no overrides. It
/// depends on nothing else, so every context of the process shares it.
static DEFAULT_KEYMAPS: std::sync::Mutex<Vec<(Platform, bool, BuiltKeymap)>> = std::sync::Mutex::new(Vec::new());

/// The keymap of `registry` and `app` on `platform` under `overrides`: built once per platform for the shared
/// registry with no overrides and no catalog, else built here.
fn keymap_for(registry: &Arc<registry::Registry>, app: Option<&AppCatalog>, platform: Platform, overrides: Option<&Arc<Overrides>>) -> BuiltKeymap {
    if app.is_some() || overrides.is_some() || !registry::Registry::is_shared(registry) {
        return build_keymap(registry, app, platform, overrides);
    }
    let mut kept = DEFAULT_KEYMAPS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let stories = registry.has_stories();
    if let Some((.., built)) = kept.iter().find(|(on, with, _)| *on == platform && *with == stories) {
        return built.clone();
    }
    let built = build_keymap(registry, None, platform, None);
    kept.push((platform, stories, built.clone()));
    built
}

/// Declares that an instance of `child` also receives the commands of `parent`, and that its shortcuts rank inside the
/// parent's. The workspace scope of a full-window dialog is a child of the view scope.
#[doc(hidden)]
pub fn declare_child(ctx: &Context, child: CommandScope, parent: CommandScope) {
    with(ctx, |board| {
        board.parents.insert(child.name(), parent.name());
    });
}

/// The innermost instance in `chain` that receives the commands of `scope`: an instance of the scope or of a child of it.
pub(crate) fn innermost(ctx: &Context, chain: &CommandChain, scope: &str) -> Option<Id> {
    with(ctx, |board| board.innermost(chain, scope))
}

/// The platform the keymap and the chords shown are read for.
pub(crate) fn platform(ctx: &Context) -> Platform {
    crate::runtime::windows::os_of(ctx)
}

/// Before each frame: last frame's state is the fallback, the app's states, gate and catalog are this frame's, and the
/// keymap follows the user's overrides and the catalog's generation. The readers of a state that changed run from
/// `begin`, which resolves again what each was shown.
pub(crate) fn set_input(ctx: &Context, input: &CommandInput) {
    let platform = platform(ctx);
    let frame = pass::root_frame_nr(ctx);
    let generation = input.catalog.as_ref().map(AppCatalog::generation);
    let shadowed = with(ctx, |board| {
        let mut states = HashMap::new();
        for (command, state) in &input.states {
            let number = board.number(&command.id);
            states.insert((number, None), state.clone());
        }
        // A state resolves from this frame's, else the last frame's.
        board.last = std::mem::replace(&mut board.this, states);
        board.modal_published_last = std::mem::take(&mut board.modal_published);
        board.gate = input.gate.clone();
        board.gate_answers.clear();
        board.app = input.catalog.clone();
        let stale = board.keymap.as_ref().is_none_or(|cache| {
            cache.platform != platform
                || cache.generation != generation
                || match (&cache.overrides, &input.overrides) {
                    (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                    (None, None) => false,
                    _ => true,
                }
        });
        let mut shadowed = Vec::new();
        if stale {
            let BuiltKeymap { keymap, edit_chords, window_shortcuts } =
                keymap_for(&board.registry, input.catalog.as_ref(), platform, input.overrides.as_ref());
            board.keymap = Some(KeymapCache { keymap, platform, overrides: input.overrides.clone(), generation, edit_chords, window_shortcuts });
            for def in input.catalog.as_ref().map_or(&[][..], AppCatalog::defs) {
                if board.registry.get(&def.command.id).is_some() && board.shadowed.insert(def.command.id.clone()) {
                    shadowed.push(def.command.id.clone());
                }
            }
        }
        board.changed = (frame, InputChanges { keymap: stale });
        shadowed
    });
    for id in shadowed {
        log::warn!("the app catalog defines {id}, which a linked command set defines: the set's definition is the command's");
    }
}

/// The chords the keymap binds to the edit commands, which the input hook removes from the raw input when the
/// focused component lists the command.
pub(crate) fn edit_chords(ctx: &Context) -> Arc<[(EditCommand, Chord)]> {
    with(ctx, |board| board.keymap.as_ref().map(|cache| Arc::clone(&cache.edit_chords)).unwrap_or_default())
}

/// The bindings the window's keymap gives `command`; none before the first frame builds one.
pub(crate) fn bindings(ctx: &Context, command: &CommandRef) -> Vec<craft_keymap::Binding> {
    with(ctx, |board| board.keymap.as_ref().map(|cache| cache.keymap.bindings(command)).unwrap_or_default())
}

/// The chord the window's keymap shows for `command` (`Keymap::shown_chord`); none before the first frame builds one.
pub(crate) fn chord(ctx: &Context, command: &CommandRef) -> Option<Chord> {
    with(ctx, |board| board.keymap.as_ref().and_then(|cache| cache.keymap.shown_chord(command)))
}

/// The command that holds `chord` in the window's keymap (`Keymap::holder`); none before the first frame builds one.
pub(crate) fn holder(ctx: &Context, chord: &Chord) -> Option<CommandRef> {
    with(ctx, |board| board.keymap.as_ref().and_then(|cache| cache.keymap.holder(chord)))
}

/// The chords bound to the window's keys, which the input hook takes before any component.
pub(crate) fn window_shortcuts(ctx: &Context) -> Arc<[(WindowShortcut, Chord)]> {
    with(ctx, |board| board.keymap.as_ref().map(|cache| Arc::clone(&cache.window_shortcuts)).unwrap_or_default())
}

/// The app-scope commands fired since the last call.
pub(crate) fn take_fired(ctx: &Context) -> Vec<Fired> {
    with(ctx, |board| std::mem::take(&mut board.out))
}

/// The commands instances took since the last call.
pub(crate) fn take_delivered(ctx: &Context) -> Vec<Fired> {
    with(ctx, |board| std::mem::take(&mut board.delivered))
}

/// Whether the component that holds key focus sits inside the instance `id`.
#[doc(hidden)]
pub fn focus_within(ctx: &Context, id: Id) -> bool {
    with(ctx, |board| board.here.focused.iter().any(|(_, chain)| chain.contains(&id)))
}

/// Records that the container `id` is an instance of `scope` in the running pass of its viewport.
pub(crate) fn register_instance(ctx: &Context, scope: CommandScope, id: Id) {
    register_in(ctx, scope, id, overlay::drawing_parent(ctx));
}

/// `register_instance` in the layer `layer`, which a replay of the registration keeps.
fn register_in(ctx: &Context, scope: CommandScope, id: Id, layer: Option<Id>) {
    crate::runtime::replay::journal(ctx, || move |ctx: &Context| register_in(ctx, scope, id, layer));
    let viewport = pass::current_viewport(ctx);
    let stamp = pass::pass_nr(ctx, viewport);
    with(ctx, |board| match board.instances.iter_mut().find(|i| i.id == id) {
        Some(instance) => {
            instance.scope = scope;
            instance.viewport = viewport;
            instance.stamp = stamp;
            instance.layer = layer;
        }
        None => board.instances.push(Instance { id, scope, viewport, stamp, layer }),
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
        board.here.modal_open = open;
        board.modal_roots.retain(|(of, _)| *of != viewport);
        board.modal_roots.extend(root.map(|root| (viewport, root)));
    });
}

/// The root of the top modal layer of `viewport`, if one is open.
pub(crate) fn modal_root(ctx: &Context, viewport: ViewportId) -> Option<Id> {
    with(ctx, |board| board.modal_roots.iter().find(|(of, _)| *of == viewport).map(|(_, root)| *root))
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
/// the gate applied outside the board's lock, and the key of the state it read.
fn resolve_in(
    ctx: &Context,
    command: &CommandRef,
    instance: Option<Id>,
    reach: Reach<'_>,
    from: Option<&Vantage>,
) -> (ResolvedCommand, Option<StateKey>) {
    let ((mut resolved, key), first_unknown, gate) = with(ctx, |board| {
        let first_unknown = board.def(&command.id).is_none() && board.unknown.insert(command.id.clone());
        // A command no state named yet has a key too, so a state given later reaches the regions that read it.
        board.number(&command.id);
        let resolved = board.resolve(ctx, command, instance, reach, from.unwrap_or(&board.here));
        // This frame's answer, else the gate to ask.
        let gate = board.gate.as_ref().map(|gate| board.gate_answers.get(command.key_text().as_ref()).copied().ok_or_else(|| gate.clone()));
        (resolved, first_unknown, gate)
    });
    if first_unknown {
        log::warn!("the command {} is in no registered command set: it is an app command with its id as its label", command.id);
    }
    let allowed = match gate {
        None => true,
        Some(Ok(answer)) => answer,
        Some(Err(gate)) => {
            let answer = gate.allows(command);
            with(ctx, |board| {
                board.gate_answers.insert(command.key(), answer);
            });
            answer
        }
    };
    if !allowed {
        resolved.enabled = false;
    }
    (resolved, key)
}

/// `command` for a surface with no component of its own: against the instance that `fire` would target.
pub(crate) fn resolve_global(ctx: &Context, command: &CommandRef) -> ResolvedCommand {
    read(ctx, Reader::Global, command)
}

/// `command` for a surface with no component of its own that closes before its command runs, from the window under
/// the modal layer `layer` (`beneath`).
pub(crate) fn resolve_beneath(ctx: &Context, layer: Id, command: &CommandRef) -> ResolvedCommand {
    read(ctx, Reader::Beneath(layer), command)
}

/// Step 1: the bound command of a component, resolved against its own instance. A component outside every instance
/// of the command's scope (a Navigator's Zoom In beside the canvas) reaches the instance a menu row of the command
/// reaches. A component inside a menu resolves it in its own chain for the instance (a panel menu's Float reaches the
/// group whose button opened it, focused or not), judged from the stop focus returns to once the menus close, which
/// gives the edits, the modal facts and the instance when the chain holds none (Cut in a text field's edit menu follows
/// the field's selection; an Edit menu row reaches the field that held focus); its activation fires that resolution.
pub(crate) fn resolve_bound(ctx: &Context, chain: &CommandChain, command: &CommandRef) -> ResolvedCommand {
    let reader = match focus::running_menu(ctx) {
        Some(menu) => Reader::InMenu(menu, chain.clone()),
        None => Reader::Chain(chain.clone()),
    };
    read(ctx, reader, command)
}

/// `command` as `reader` is shown it, noted as its read.
fn read(ctx: &Context, reader: Reader, command: &CommandRef) -> ResolvedCommand {
    let (resolved, key) = resolve_as(ctx, &reader, command);
    note_read(ctx, key, reader, &resolved);
    resolved
}

/// `command` from the vantage of `reader`, and the key of the state it read. The readers' first resolution and the one
/// `begin` makes again both come from here.
fn resolve_as(ctx: &Context, reader: &Reader, command: &CommandRef) -> (ResolvedCommand, Option<StateKey>) {
    match reader {
        Reader::Global => resolve_from(ctx, command, None),
        Reader::Beneath(layer) => {
            let from = beneath(ctx, *layer);
            resolve_from(ctx, command, Some(&from))
        }
        Reader::Chain(chain) => resolve_chain(ctx, command, chain),
        Reader::InMenu(menu, chain) => {
            let from = out_of_menu(ctx, *menu);
            let instance = with(ctx, |board| {
                board.innermost(chain, board.scope_of(command)).or_else(|| board.target_of(ctx, command, from.as_ref().unwrap_or(&board.here)).1)
            });
            resolve_in(ctx, command, instance, Reach::Chain(chain), from.as_ref())
        }
    }
}

/// `command` against the instances of `chain`.
fn resolve_chain(ctx: &Context, command: &CommandRef, chain: &CommandChain) -> (ResolvedCommand, Option<StateKey>) {
    let instance = with(ctx, |board| board.innermost(chain, board.scope_of(command)).or_else(|| board.target_of(ctx, command, &board.here).1));
    resolve_in(ctx, command, instance, Reach::Chain(chain), None)
}

fn resolve_from(ctx: &Context, command: &CommandRef, from: Option<&Vantage>) -> (ResolvedCommand, Option<StateKey>) {
    let (_, instance) = with(ctx, |board| board.target_of(ctx, command, from.unwrap_or(&board.here)));
    resolve_in(ctx, command, instance, Reach::Focus, from)
}

/// fx.F23: resolves every command a reader of `viewport` was shown again, from that reader's own vantage under this
/// pass's gate, states, modal facts and instances, and notes an edit of the state key of each whose resolution differs
/// in any shown part (label, chord, enabled, checked, marked, hint), so a settled region that shows it runs; the entry
/// keeps the new resolution. Readers of regions that did not run keep their entries: a settled region is the reader
/// this exists for. It reads no input: a second pass of a frame compares against what the first pass's runs were shown
/// and what the first pass's call kept, so it wakes only the readers whose resolution the first pass changed (a modal
/// layer it opened or closed), once.
fn wake_shown(ctx: &Context, viewport: ViewportId) {
    let mut shown = with(ctx, |board| std::mem::take(&mut board.shown));
    let mut changed = Vec::new();
    for ((key, of), readers) in &mut shown {
        if *of != viewport {
            continue;
        }
        // A reader beneath a layer or inside a menu that is no longer shown is gone with it.
        readers.retain(|entry| match entry.reader {
            Reader::Beneath(layer) | Reader::InMenu(layer, _) => overlay::is_open(ctx, layer),
            _ => true,
        });
        for entry in readers.iter_mut() {
            // The gate is app code and may resolve commands itself, so no board lock is held here.
            let (resolved, _) = resolve_as(ctx, &entry.reader, &entry.resolved.command);
            if resolved != entry.resolved {
                entry.resolved = resolved;
                changed.push(*key);
            }
        }
    }
    shown.retain(|_, readers| !readers.is_empty());
    with(ctx, |board| {
        // Reads the gate made while the entries were out of the board.
        let added = std::mem::replace(&mut board.shown, shown);
        for ((key, of), readers) in added {
            for entry in readers {
                keep_shown(&mut board.shown, key, of, entry.reader, &entry.resolved);
            }
        }
    });
    for (number, instance) in changed {
        lazy::note_edit(ctx, BindKey::Command(number, instance));
    }
}

/// Step-1 resolution for a context in `chain`; `None` when the command's scope has no instance in the chain.
pub(crate) fn resolve_scoped(ctx: &Context, chain: &CommandChain, command: &CommandRef) -> Option<ResolvedCommand> {
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
    // An edit command goes to the focused component that lists it; otherwise it goes to the instance that declared
    // it or, with none, stays an app command.
    if fired.phase != FirePhase::Release
        && let Some(edit) = with(ctx, |board| board.registry.edit_of(&fired.command.id))
        && keys::queue_edit(ctx, edit)
    {
        return;
    }
    with(ctx, |board| if fired.target.is_some() { board.queue.push(fired) } else { board.out.push(fired) });
    // A scoped entry is taken next pass, so the root viewport runs one.
    repaint::request_of(ctx, ViewportId::ROOT);
}

/// An undo or redo a text part or editor had nothing to do for goes to the instance that declares the command or,
/// with none, to the app command of the same id (11.3).
pub(crate) fn fall_through(ctx: &Context, command: craft_behavior::EditCommand) {
    let Some(command) = EditCommands::of(command) else { return };
    let command = CommandRef::from(command);
    let target = with(ctx, |board| board.resolve(ctx, &command, None, Reach::Focus, &board.here).0.instance);
    let fired = Fired { target, ..Fired::new(command, FireSource::Key, FirePhase::Press, pass::now(ctx)) };
    with(ctx, |board| if fired.target.is_some() { board.queue.push(fired) } else { board.out.push(fired) });
    repaint::request_of(ctx, ViewportId::ROOT);
}

/// Step 17: the command of a component that was activated and enabled.
pub(crate) fn fire_bound(ctx: &Context, command: &ResolvedCommand) {
    fire_resolved(ctx, command, FireSource::Pointer);
}

/// Queues `command` for its instance with `source`; false when disabled.
pub(crate) fn fire_resolved(ctx: &Context, command: &ResolvedCommand, source: FireSource) -> bool {
    if !command.enabled || !crate::runtime::measure::effect(ctx, "board::fire_resolved") {
        return false;
    }
    queue(ctx, Fired { target: command.instance, ..Fired::new(command.command.clone(), source, FirePhase::Press, pass::now(ctx)) });
    true
}

pub(crate) fn fire_from(ctx: &Context, command: &CommandRef, source: FireSource) -> bool {
    fire_in(ctx, command, source, None)
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
    let (resolved, _) = resolve_in(ctx, command, instance, Reach::Focus, from);
    if !resolved.enabled {
        return false;
    }
    // An app-scope command a modal layer's instance declares is delivered to that instance.
    let target = if app { resolved.instance } else { instance };
    queue(ctx, Fired { target, ..Fired::new(command.clone(), source, FirePhase::Press, pass::now(ctx)) });
    true
}

/// Publishes `state` of `command` for the innermost instance in `chain`. A state other than the one resolved so far in
/// the frame is an edit of it: a settled region that read it runs, and so does the pass, when something read it
/// earlier in the pass.
///
/// A publish is a board effect, which a measuring run skips as it skips registration: the real run of the same content
/// publishes, and its edit check sees the reads the measuring runs made.
pub(crate) fn publish(ctx: &Context, chain: &CommandChain, command: &CommandRef, state: CommandState) {
    if !measure::effect(ctx, "commands::publish") {
        return;
    }
    replay::journal(ctx, || {
        let (chain, command, state) = (chain.clone(), command.clone(), state.clone());
        move |ctx: &Context| publish(ctx, &chain, &command, state.clone())
    });
    let changed = with(ctx, |board| {
        let scope = board.scope_of(command);
        let declared = board.def(&command.id).is_some_and(|def| def.modal == ModalRule::Declared);
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
        if in_modal {
            board.modal_published.insert(key);
        }
        let changed = !same_state(board.this.get(&key).or_else(|| board.last.get(&key)), Some(&state));
        board.this.insert(key, state);
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
    let (resolved, _) = resolve_in(ctx, &command, instance, Reach::Chain(around), None);
    if resolved.enabled { S::from_ref(&command) } else { None }
}

/// Whether the keymap binds `chord` to an enabled scoped command of the focused component's command chain, matched
/// as the key stage matches the press. A key the focused stop would activate with is left to that command.
pub(crate) fn binds_scoped(ctx: &Context, chord: &Chord) -> bool {
    let viewport = pass::current_viewport(ctx);
    let focused = focus::record(ctx, viewport).focused;
    let layout = keys::layout_of(ctx);
    let found = with(ctx, |board| {
        let chain: Vec<Id> =
            board.chains.get(&viewport).filter(|(id, _)| Some(*id) == focused).map(|(_, chain)| chain.ids().collect()).unwrap_or_default();
        let keys = board.key_scopes(&chain)?;
        let hit = keys.press(&layout, &chord_input(chord), false, None)?;
        // The lookup also ranks panels that share with the focused one and, with fallthrough, every other panel: only
        // a command of a scope around the focus counts.
        let command = std::iter::once(&hit.command).chain(&hit.cycle).find(|command| keys.scopes.contains(&board.scope_of(command)))?.clone();
        let (_, instance) = board.target_of(ctx, &command, &board.here);
        Some((command, instance))
    });
    found.is_some_and(|(command, instance)| resolve_in(ctx, &command, instance, Reach::Focus, None).0.enabled)
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
        match focus::focused_rec(ctx).and_then(|rec| focus::out_of_menus(ctx, rec.scope)) {
            Some(out) if out.bar => Some(Self::Bar(out.to.0)),
            Some(out) => Some(Self::Stop(out.to)),
            None => focus::bar_return_of(ctx, focused).map(|(held, _)| Self::Bar(held)),
        }
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

/// H7: the key stage's shortcut events, matched against the keymap in the scopes around the focused component,
/// through the gate, to their commands.
pub(crate) fn begin(ctx: &Context) {
    let (arrived, changed) = with(ctx, |board| board.changed.clone());
    if arrived == pass::root_frame_nr(ctx) && changed.keymap {
        lazy::unsettle(ctx);
    }
    let viewport = pass::current_viewport(ctx);
    let focused = focus::record(ctx, viewport).focused;
    let shortcuts = keys::take_shortcuts(ctx);
    let now = pass::now(ctx);
    let held_open = sampler::held_canvas(ctx);
    // A shortcut pressed in a menu or on a menu bar matches where the focus returns to once the menus close.
    let returning = focused.filter(|_| !shortcuts.keys.is_empty()).and_then(|focused| Returning::of(ctx, focused));
    let shell = frame::shell_root(ctx, viewport);
    let taken = with(ctx, |board| {
        board.held_open = held_open;
        let returned = returning
            .map(Returning::id)
            .and_then(|held| [board.before.get(&viewport), board.chains.get(&viewport)].into_iter().flatten().find(|(id, _)| *id == held))
            .map(|(_, chain)| chain.ids().collect());
        let chain: Vec<Id> = returned.unwrap_or_else(|| {
            board.chains.get(&viewport).filter(|(id, _)| Some(*id) == focused).map(|(_, chain)| chain.ids().collect()).unwrap_or_default()
        });
        // Nothing focused inside a modal: the modal's root takes the commands, as a plain Enter goes to its default button.
        let chain = match (chain.is_empty() && focused.is_none(), board.modal_roots.iter().find(|(of, _)| *of == viewport)) {
            (true, Some((_, root))) => vec![*root],
            _ => chain,
        };
        if !chain.is_empty() {
            board.last_focused.clone_from(&chain);
        }
        // Nothing focused outside a modal: the window's shell root takes the commands, so a window command's key comes
        // before an app command's. Nothing holds focus there, so the chain last focused stays.
        let chain = if chain.is_empty() && focused.is_none() { shell.into_iter().collect() } else { chain };
        match board.here.focused.iter_mut().find(|(v, _)| *v == viewport) {
            Some(entry) => entry.1.clone_from(&chain),
            None => board.here.focused.push((viewport, chain.clone())),
        }
        board.here.dialog_keys = shortcuts.dialog;
        board.here.inside_modal = Board::inside_modal_of(&chain, |id| board.is_modal_root(id));
        if shortcuts.keys.is_empty() {
            return None;
        }
        let mut keys = board.key_scopes(&chain)?;
        // The held-open canvas's scope is around the focus for matching: its commands fire from the dialog.
        let held_scope = held_open.and_then(|id| board.instances.iter().find(|i| i.id == id)).map(|i| i.scope.name()).filter(|scope| !is_app(scope));
        if let Some(scope) = held_scope
            && !keys.scopes.contains(&scope)
        {
            keys.scopes.push(scope);
        }
        Some((keys, std::mem::take(&mut board.matcher)))
    });
    // With this pass's vantage set, what the readers were shown is resolved again before any of them runs.
    wake_shown(ctx, viewport);
    let Some((keys, mut matcher)) = taken else { return };
    let mut fired = Vec::new();
    for shortcut in &shortcuts.keys {
        // When the key event arrived: a frame may run it later, and a double press and a hold read the key's own times.
        let time = shortcut.at.unwrap_or(now);
        match shortcut.phase {
            FirePhase::Release => {
                let key = shortcut.input.physical.or(shortcut.input.typed);
                let held = with(ctx, |board| key.and_then(|key| board.held.iter().position(|held| held.key == key)).map(|at| board.held.remove(at)));
                if let Some(held) = held {
                    fired.push(Fired {
                        command: held.command,
                        target: held.target,
                        source: FireSource::Key,
                        phase: FirePhase::Release,
                        cycle: false,
                        alt_variant: false,
                        time,
                    });
                }
            }
            _ => {
                let Some(hit) = keys.press(&shortcuts.layout, &shortcut.input, shortcuts.typing && !shortcut.canvas, Some((&mut matcher, time)))
                else {
                    continue;
                };
                let (app, instance) = with(ctx, |board| board.target_of(ctx, &hit.command, &board.here));
                let (resolved, _) = resolve_in(ctx, &hit.command, instance, Reach::Focus, None);
                if !resolved.enabled {
                    continue;
                }
                // An app-scope command a modal layer's instance declares is delivered to that instance.
                let target = if app { resolved.instance } else { instance };
                let key = shortcut.input.physical.or(shortcut.input.typed);
                with(ctx, |board| {
                    if let Some(key) = key.filter(|_| {
                        board.def(&hit.command.id).is_some_and(|def| matches!(def.kind, CommandKind::Momentary | CommandKind::Tool { .. }))
                    }) {
                        board.held.retain(|held| held.key != key);
                        board.held.push(Held { key, command: hit.command.clone(), target });
                    }
                });
                fired.push(Fired {
                    command: hit.command,
                    target,
                    source: FireSource::Key,
                    phase: FirePhase::Press,
                    cycle: hit.cycle.len() > 1,
                    alt_variant: hit.alt_variant,
                    time,
                });
            }
        }
    }
    with(ctx, |board| board.matcher = matcher);
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
                .filter_map(|target| board.instances.iter().find(|instance| instance.id == target)?.layer)
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
    with(ctx, |board| {
        let live = pass::live_viewports(ctx);
        let alive: Vec<bool> = board.instances.iter().map(|i| board.live(ctx, i) && live.contains(&i.viewport)).collect();
        let mut keep = alive.iter();
        board.instances.retain(|_| keep.next().copied().unwrap_or(false));
        let ids: Vec<Id> = board.instances.iter().map(|i| i.id).collect();
        board.queue.retain(|fired| fired.target.is_some_and(|id| ids.contains(&id)));
        board.held.retain(|held| held.target.is_none_or(|id| ids.contains(&id)));
        board.chains.retain(|viewport, _| live.contains(viewport));
        board.before.retain(|viewport, _| live.contains(viewport));
        board.here.focused.retain(|(viewport, _)| live.contains(viewport));
        board.shown.retain(|(_, viewport), readers| {
            readers.retain(|entry| match &entry.reader {
                Reader::Chain(chain) | Reader::InMenu(_, chain) => chain.innermost_any().is_none_or(|id| ids.contains(&id)),
                _ => true,
            });
            live.contains(viewport) && !readers.is_empty()
        });
    });
}
