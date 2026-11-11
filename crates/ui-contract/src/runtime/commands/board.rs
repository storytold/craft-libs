//! The board: state by `(command, instance)`, the instances of each scope, the queue, resolution, firing and the
//! dispatcher that turns the key stage's shortcut events into fired commands (spec 11.3).

use super::{
    CommandChain, CommandGate, CommandInput, CommandScope, CommandSet, CommandState, EditCommands, FirePhase, FireSource, Fired, ResolvedCommand,
    registry,
};
use crate::runtime::egui::memory::Service;
use crate::runtime::egui::{pass, repaint};
use crate::runtime::{focus, keys, sampler};
use craft_behavior::{EditCommand, pick_instance, resolve_flags};
use craft_keymap::{Active, Chord, CommandKind, CommandRef, Keymap, Lookup, Matcher, ModalRule, Overrides, Platform, Press, Scopes};
use egui::{Context, Id, ViewportId};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

type StateKey = (u32, Option<Id>);

/// How a command with `ModalRule::Declared` reaches into the top modal layer.
#[derive(Clone, Copy)]
enum Declaration {
    /// An instance publishes the command's state.
    Instance(Id),
    /// The focused component takes the command as an edit.
    Focused,
}

/// A container instance that receives scoped commands.
struct Instance {
    id: Id,
    scope: CommandScope,
    viewport: ViewportId,
    /// The pass of its viewport that last registered it.
    stamp: u64,
}

struct KeymapCache {
    keymap: Arc<Keymap>,
    platform: Platform,
    overrides: Option<Arc<Overrides>>,
    /// The chords bound to the edit commands, which the input hook removes from the raw input when the focused
    /// component lists the command.
    edit_chords: Arc<[(EditCommand, Chord)]>,
}

/// A momentary command whose key is down: its release fires the command again.
struct Held {
    key: craft_keymap::Key,
    command: CommandRef,
    target: Option<Id>,
}

#[derive(Default)]
struct Board {
    registry: Arc<registry::Registry>,
    keymap: Option<KeymapCache>,
    gate: Option<CommandGate>,
    /// Command ids by number, so a state lookup allocates nothing.
    numbers: HashMap<String, u32>,
    this: HashMap<StateKey, CommandState>,
    last: HashMap<StateKey, CommandState>,
    /// Scoped commands waiting for their instance.
    queue: Vec<Fired>,
    /// App-scope commands for the next `FrameOutput`.
    out: Vec<Fired>,
    instances: Vec<Instance>,
    /// The focused component of each viewport's last step 12 and the instances around it.
    chains: HashMap<ViewportId, (Id, CommandChain)>,
    /// The instances around the component that holds key focus, per viewport, as the viewport's begin hook read them.
    focused: Vec<(ViewportId, Vec<Id>)>,
    /// The chain of the last pass in which any component held key focus.
    last_focused: Vec<Id>,
    matcher: Matcher,
    held: Vec<Held>,
    modal_open: bool,
    /// The top modal layer's root container of each viewport that has one.
    modal_roots: Vec<(ViewportId, Id)>,
    /// The instances around the focus from the innermost out to the root of the modal layer the focus is in: the
    /// instances `ModalRule::Blocked` does not block. Empty while the focus is outside every modal layer.
    inside_modal: Vec<Id>,
    /// The states published this pass, and the pass before, by an instance inside the top modal layer.
    modal_published: HashSet<StateKey>,
    modal_published_last: HashSet<StateKey>,
    /// The canvas a modal layer keeps open (`runtime::sampler`): its scope's commands reach it while the focus is in
    /// the dialog.
    held_open: Option<Id>,
    /// A dialog owns the keys in the running pass.
    dialog_keys: bool,
    /// Bound command ids that no set registers, already logged.
    unknown: HashSet<String>,
    /// The scope each child scope is declared inside: an instance of the child is also an instance of the parent.
    parents: HashMap<&'static str, &'static str>,
}

fn with<R>(ctx: &Context, f: impl FnOnce(&mut Board) -> R) -> R {
    Service::<Board>::run(ctx, f)
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

    fn is_live(&self, ctx: &Context, id: Id) -> bool {
        self.instances.iter().any(|i| i.id == id && self.live(ctx, i))
    }

    /// The scope name of `command`; commands the registry does not know belong to the application.
    fn scope_of(&self, command: &CommandRef) -> &str {
        self.registry.get(&command.id).map_or(CommandScope::APP.name(), |def| def.scope.as_str())
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
    fn pick(&self, ctx: &Context, scope: &str) -> Option<Id> {
        // The held-open canvas ranks below every instance around the focus.
        let focused: Vec<Id> = self.held_open.into_iter().chain(self.focused.iter().flat_map(|(_, chain)| chain.iter().copied())).collect();
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

    /// Where `command` goes when a source with no component of its own fires it: `None` for an app-scope command,
    /// else the picked instance, which may not exist.
    fn target_of(&self, ctx: &Context, command: &CommandRef) -> (bool, Option<Id>) {
        let scope = self.scope_of(command);
        let app = is_app(scope);
        (app, if app { None } else { self.pick(ctx, scope) })
    }

    /// Who answers for `command` ahead of the app: an instance inside the top modal layer that published its state,
    /// else the focused component when it takes the command as an edit (inside a modal layer, only when the focus
    /// is in it).
    fn declaring(&self, ctx: &Context, command: &CommandRef, dialog_open: bool) -> Option<Declaration> {
        let number = *self.numbers.get(command.id.as_str())?;
        let published = self.modal_published.iter().chain(&self.modal_published_last).find(|(published, _)| *published == number);
        if let Some(id) = published.and_then(|(_, id)| *id).filter(|_| dialog_open) {
            return Some(Declaration::Instance(id));
        }
        let edit = self.registry.edit_of(&command.id)?;
        let record = focus::record(ctx, pass::current_viewport(ctx));
        let in_reach = !dialog_open || !self.inside_modal.is_empty();
        (in_reach && record.keys.is_some_and(|keys| keys.edits.contains(edit))).then_some(Declaration::Focused)
    }

    /// `command` as seen by `instance`, or by the app for an app-scope command, before the gate. Also whether this is
    /// the first time a command no set registers was asked for.
    fn resolve(&mut self, ctx: &Context, command: &CommandRef, instance: Option<Id>) -> (ResolvedCommand, bool) {
        let def = self.registry.get(&command.id);
        let first_unknown = def.is_none() && self.unknown.insert(command.id.clone());
        let app = is_app(self.scope_of(command));
        let instance = if app { None } else { instance };
        let dialog_open = self.modal_open || self.dialog_keys;
        // A command no set registers is never blocked.
        let rule = def.map(|def| def.modal);
        // The instance inside the top modal that declares the command, which then answers for it.
        let declaring = (rule == Some(ModalRule::Declared)).then(|| self.declaring(ctx, command, dialog_open)).flatten();
        // The declaring instance answers for the command: it is where the command is delivered. The focused component
        // answers for itself, so the app's state of the command does not apply.
        let instance = match declaring {
            Some(Declaration::Instance(id)) => Some(id),
            _ => instance,
        };
        let key = match declaring {
            Some(Declaration::Instance(id)) => self.numbers.get(command.id.as_str()).map(|&number| (number, Some(id))),
            Some(Declaration::Focused) => None,
            None => self.state_key(command, instance),
        };
        let this = key.and_then(|key| self.this.get(&key));
        let last = key.and_then(|key| self.last.get(&key));
        let state = this.or(last);
        let flags = resolve_flags(this.map(CommandState::flags), last.map(CommandState::flags));
        let reached = declaring.is_some() || (!app && instance.is_some_and(|id| self.inside_modal.contains(&id)));
        let modal = !(dialog_open && !reached && matches!(rule, Some(ModalRule::Blocked | ModalRule::Declared)));
        let live = app || instance.is_some_and(|id| self.is_live(ctx, id));
        let chord =
            self.keymap.as_ref().and_then(|cache| cache.keymap.bindings(command).into_iter().find(|b| !b.trigger.double).map(|b| b.trigger.chord));
        let resolved = ResolvedCommand {
            label: state.and_then(|s| s.title_ref().cloned()).unwrap_or_else(|| Arc::from(def.map_or(command.id.as_str(), |def| def.label.as_str()))),
            icon: def.and_then(|def| def.icon.clone()),
            chord,
            enabled: flags.enabled && modal && live,
            checked: flags.checked,
            marked: state.is_some_and(CommandState::is_marked),
            hint: state.and_then(|s| s.hint_ref().cloned()),
            kind: def.map_or(CommandKind::Command, |def| def.kind.clone()),
            command: command.clone(),
            instance,
        };
        (resolved, first_unknown)
    }
}

fn is_app(scope: &str) -> bool {
    scope == CommandScope::APP.name()
}

/// Loads the linked command sets.
pub(crate) fn install(ctx: &Context) -> Result<(), registry::CommandDefError> {
    let registry = registry::Registry::shared()?;
    with(ctx, |board| board.registry = registry);
    Ok(())
}

/// A keymap and the chords it binds to the edit commands.
type BuiltKeymap = (Arc<Keymap>, Arc<[(EditCommand, Chord)]>);

/// The keymap of `registry` on `platform` under `overrides`, and the chords it binds to the edit commands.
fn build_keymap(registry: &registry::Registry, platform: Platform, overrides: Option<&Arc<Overrides>>) -> BuiltKeymap {
    let mut scopes = Scopes::new();
    for def in registry.defs() {
        if !def.scope.is_app() {
            scopes = scopes.with_panel(def.scope.as_str());
        }
    }
    let mut keymap = Keymap::new(registry.defs_for(platform), scopes, platform);
    if let Some(overrides) = overrides {
        keymap.set_overrides((**overrides).clone());
    }
    let edit_chords = registry
        .defs()
        .iter()
        .filter_map(|def| registry.edit_of(&def.command.id).map(|edit| (edit, &def.command)))
        .flat_map(|(edit, command)| {
            keymap.bindings(command).into_iter().filter(|binding| !binding.trigger.double).map(move |binding| (edit, binding.trigger.chord))
        })
        .collect();
    (Arc::new(keymap), edit_chords)
}

/// The keymap of the shared registry on each platform with no overrides. It depends on nothing else, so every
/// context of the process shares it.
static DEFAULT_KEYMAPS: std::sync::Mutex<Vec<(Platform, BuiltKeymap)>> = std::sync::Mutex::new(Vec::new());

/// The keymap of `registry` on `platform` under `overrides`: built once per platform for the shared registry
/// with no overrides, else built here.
fn keymap_for(registry: &Arc<registry::Registry>, platform: Platform, overrides: Option<&Arc<Overrides>>) -> BuiltKeymap {
    if overrides.is_some() || !registry::Registry::is_shared(registry) {
        return build_keymap(registry, platform, overrides);
    }
    let mut kept = DEFAULT_KEYMAPS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, built)) = kept.iter().find(|(on, _)| *on == platform) {
        return built.clone();
    }
    let built = build_keymap(registry, platform, None);
    kept.push((platform, built.clone()));
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

/// Before each frame: last frame's state is the fallback, the app's states and gate are this frame's, and the
/// keymap follows the user's overrides.
pub(crate) fn set_input(ctx: &Context, input: &CommandInput) {
    let platform = platform(ctx);
    with(ctx, |board| {
        board.last = std::mem::take(&mut board.this);
        board.modal_published_last = std::mem::take(&mut board.modal_published);
        for (command, state) in &input.states {
            let number = board.number(&command.id);
            board.this.insert((number, None), state.clone());
        }
        board.gate = input.gate.clone();
        let stale = board.keymap.as_ref().is_none_or(|cache| {
            cache.platform != platform
                || match (&cache.overrides, &input.overrides) {
                    (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                    (None, None) => false,
                    _ => true,
                }
        });
        if stale {
            let (keymap, edit_chords) = keymap_for(&board.registry, platform, input.overrides.as_ref());
            board.keymap = Some(KeymapCache { keymap, platform, overrides: input.overrides.clone(), edit_chords });
        }
    });
}

/// The chords the keymap binds to the edit commands, which the input hook removes from the raw input when the
/// focused component lists the command.
pub(crate) fn edit_chords(ctx: &Context) -> Arc<[(EditCommand, Chord)]> {
    with(ctx, |board| board.keymap.as_ref().map(|cache| Arc::clone(&cache.edit_chords)).unwrap_or_default())
}

/// The app-scope commands fired since the last call.
pub(crate) fn take_fired(ctx: &Context) -> Vec<Fired> {
    with(ctx, |board| std::mem::take(&mut board.out))
}

/// Whether the component that holds key focus sits inside the instance `id`.
#[doc(hidden)]
pub fn focus_within(ctx: &Context, id: Id) -> bool {
    with(ctx, |board| board.focused.iter().any(|(_, chain)| chain.contains(&id)))
}

/// Records that the container `id` is an instance of `scope` in the running pass of its viewport.
pub(crate) fn register_instance(ctx: &Context, scope: CommandScope, id: Id) {
    let viewport = pass::current_viewport(ctx);
    let stamp = pass::pass_nr(ctx, viewport);
    with(ctx, |board| match board.instances.iter_mut().find(|i| i.id == id) {
        Some(instance) => {
            instance.scope = scope;
            instance.viewport = viewport;
            instance.stamp = stamp;
        }
        None => board.instances.push(Instance { id, scope, viewport, stamp }),
    });
}

/// Records the instances around the component `id`, which holds key focus in the running pass of its viewport. The
/// next begin hook of the viewport reads the chain of the component its focus record names.
pub(crate) fn note_focus(ctx: &Context, id: Id, chain: &CommandChain) {
    let viewport = pass::current_viewport(ctx);
    with(ctx, |board| {
        board.chains.insert(viewport, (id, chain.clone()));
    });
}

pub(crate) fn set_modal_open(ctx: &Context, open: bool, viewport: ViewportId, root: Option<Id>) {
    with(ctx, |board| {
        board.modal_open = open;
        board.modal_roots.retain(|(of, _)| *of != viewport);
        board.modal_roots.extend(root.map(|root| (viewport, root)));
    });
}

/// The root of the top modal layer of `viewport`, if one is open.
pub(crate) fn modal_root(ctx: &Context, viewport: ViewportId) -> Option<Id> {
    with(ctx, |board| board.modal_roots.iter().find(|(of, _)| *of == viewport).map(|(_, root)| *root))
}

/// `command` as seen by `instance` (`None` for the app), with the gate applied outside the board's lock.
fn resolve_in(ctx: &Context, command: &CommandRef, instance: Option<Id>) -> ResolvedCommand {
    let (mut resolved, first_unknown, gate) = with(ctx, |board| {
        let (resolved, first_unknown) = board.resolve(ctx, command, instance);
        (resolved, first_unknown, board.gate.clone())
    });
    if first_unknown {
        log::warn!("the command {} is in no registered command set: it is an app command with its id as its label", command.id);
    }
    if gate.is_some_and(|gate| !gate.allows(command)) {
        resolved.enabled = false;
    }
    resolved
}

/// `command` for a surface with no component of its own: against the instance that `fire` would target.
pub(crate) fn resolve_global(ctx: &Context, command: &CommandRef) -> ResolvedCommand {
    let (_, instance) = with(ctx, |board| board.target_of(ctx, command));
    resolve_in(ctx, command, instance)
}

/// Step 1: the bound command of a component, resolved against its own instance. A component outside every instance
/// of the command's scope (a Navigator's Zoom In beside the canvas) reaches the instance a menu row of the command
/// reaches.
pub(crate) fn resolve_bound(ctx: &Context, chain: &CommandChain, command: &CommandRef) -> ResolvedCommand {
    let instance = with(ctx, |board| board.innermost(chain, board.scope_of(command)).or_else(|| board.target_of(ctx, command).1));
    resolve_in(ctx, command, instance)
}

/// Step-1 resolution for a context in `chain`; `None` when the command's scope has no instance in the chain.
pub(crate) fn resolve_scoped(ctx: &Context, chain: &CommandChain, command: &CommandRef) -> Option<ResolvedCommand> {
    let (found, app) = with(ctx, |board| {
        let scope = board.scope_of(command);
        (board.innermost(chain, scope).is_some(), is_app(scope))
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
    let target = with(ctx, |board| board.resolve(ctx, &command, None).0.instance);
    let fired = Fired { command, target, source: FireSource::Key, phase: FirePhase::Press, cycle: false, alt_variant: false };
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
    queue(
        ctx,
        Fired { command: command.command.clone(), target: command.instance, source, phase: FirePhase::Press, cycle: false, alt_variant: false },
    );
    true
}

pub(crate) fn fire_from(ctx: &Context, command: &CommandRef, source: FireSource) -> bool {
    let (app, instance) = with(ctx, |board| board.target_of(ctx, command));
    if !app && instance.is_none() {
        return false;
    }
    let resolved = resolve_in(ctx, command, instance);
    if !resolved.enabled {
        return false;
    }
    // An app-scope command a modal layer's instance declares is delivered to that instance.
    let target = if app { resolved.instance } else { instance };
    queue(ctx, Fired { command: command.clone(), target, source, phase: FirePhase::Press, cycle: false, alt_variant: false });
    true
}

pub(crate) fn publish(ctx: &Context, chain: &CommandChain, command: &CommandRef, state: CommandState) {
    with(ctx, |board| {
        let scope = board.scope_of(command);
        let declared = board.registry.get(&command.id).is_some_and(|def| def.modal == ModalRule::Declared);
        debug_assert!(
            !is_app(scope) || declared,
            "{command} is an app-scope command: its state comes from `FrameInput.commands`, unless the command is declared by an instance"
        );
        // A declared app-scope command is published by the innermost instance around the caller, inside whichever
        // modal layer it sits in.
        let instance = if is_app(scope) { chain.innermost_any() } else { board.innermost(chain, scope) };
        if let Some(instance) = instance {
            let number = board.number(&command.id);
            // The pass that publishes is the one that knows the layers around the caller.
            let in_modal = chain.ids().any(|id| board.modal_roots.iter().any(|(_, root)| *root == id));
            if in_modal {
                board.modal_published.insert((number, Some(instance)));
            }
            board.this.insert((number, Some(instance)), state);
        }
    });
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
        if app {
            let at = board.out.iter().position(|fired| from_ref(&fired.command).is_some())?;
            let fired = board.out.remove(at);
            return Some((from_ref(&fired.command)?, fired));
        }
        let at = board.queue.iter().position(|fired| fired.target == Some(instance) && from_ref(&fired.command).is_some())?;
        let fired = board.queue.remove(at);
        Some((from_ref(&fired.command)?, fired))
    })
}

/// The next command of set `S` routed to the instance of its scope at `at` (a canvas that is one itself) or, when
/// `at` is no instance of it, to the innermost instance in `around`, the chain of the context `at` was shown in.
#[doc(hidden)]
pub fn take_around<S: CommandSet>(ctx: &Context, at: Id, around: &CommandChain) -> Option<(S, Fired)> {
    let scope = S::SCOPE.name();
    let (own, nearest) =
        with(ctx, |board| (board.instances.iter().any(|i| i.id == at && board.within(i.scope.name(), scope)), board.innermost(around, scope)));
    take(ctx, if own { Some(at) } else { nearest }, is_app(scope), S::from_ref)
}

/// H7: the key stage's shortcut events, matched against the keymap in the scopes around the focused component,
/// through the gate, to their commands.
/// Whether the keymap binds `chord` to a scoped command of the focused component's command chain. A key the focused
/// stop would activate with is left to that command.
pub(crate) fn binds_scoped(ctx: &Context, chord: &Chord) -> bool {
    let viewport = pass::current_viewport(ctx);
    let focused = focus::record(ctx, viewport).focused;
    with(ctx, |board| {
        let Some(keymap) = board.keymap.as_ref().map(|cache| Arc::clone(&cache.keymap)) else { return false };
        let chain: Vec<Id> =
            board.chains.get(&viewport).filter(|(id, _)| Some(*id) == focused).map(|(_, chain)| chain.ids().collect()).unwrap_or_default();
        let scopes = board.scopes_around(&chain);
        let (panel, outer) = scopes.split_first().map_or((None, &[][..]), |(panel, outer)| (Some(*panel), outer));
        let active = Active { panel, contexts: &[], typing: false };
        match keymap.lookup_nested(chord, false, &active, outer) {
            Lookup::Run(command) => !is_app(board.scope_of(&command)),
            Lookup::Cycle(commands) => commands.iter().any(|command| !is_app(board.scope_of(command))),
            _ => false,
        }
    })
}

pub(crate) fn begin(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    let focused = focus::record(ctx, viewport).focused;
    let shortcuts = keys::take_shortcuts(ctx);
    let now = pass::now(ctx);
    let held_open = sampler::held_canvas(ctx);
    let taken = with(ctx, |board| {
        board.held_open = held_open;
        let chain: Vec<Id> =
            board.chains.get(&viewport).filter(|(id, _)| Some(*id) == focused).map(|(_, chain)| chain.ids().collect()).unwrap_or_default();
        // Nothing focused inside a modal: the modal's root takes the commands, as a plain Enter goes to its default button.
        let chain = match (chain.is_empty() && focused.is_none(), board.modal_roots.iter().find(|(of, _)| *of == viewport)) {
            (true, Some((_, root))) => vec![*root],
            _ => chain,
        };
        if !chain.is_empty() {
            board.last_focused.clone_from(&chain);
        }
        match board.focused.iter_mut().find(|(v, _)| *v == viewport) {
            Some(entry) => entry.1.clone_from(&chain),
            None => board.focused.push((viewport, chain.clone())),
        }
        board.dialog_keys = shortcuts.dialog;
        let roots: Vec<Id> = board.modal_roots.iter().map(|(_, root)| *root).collect();
        board.inside_modal = chain.iter().position(|id| roots.contains(id)).map_or_else(Vec::new, |at| chain[..=at].to_vec());
        if shortcuts.keys.is_empty() {
            return None;
        }
        let keymap = board.keymap.as_ref().map(|cache| Arc::clone(&cache.keymap))?;
        let mut scopes = board.scopes_around(&chain);
        // The held-open canvas's scope is around the focus for matching: its commands fire from the dialog.
        let held_scope = held_open.and_then(|id| board.instances.iter().find(|i| i.id == id)).map(|i| i.scope.name()).filter(|scope| !is_app(scope));
        if let Some(scope) = held_scope
            && !scopes.contains(&scope)
        {
            scopes.push(scope);
        }
        Some((keymap, scopes, std::mem::take(&mut board.matcher)))
    });
    let Some((keymap, scopes, mut matcher)) = taken else { return };
    let (panel, outer) = scopes.split_first().map_or((None, &[][..]), |(panel, outer)| (Some(*panel), outer));
    let active = Active { panel, contexts: &[], typing: shortcuts.typing };
    let mut fired = Vec::new();
    for shortcut in &shortcuts.keys {
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
                    });
                }
            }
            _ => {
                let Some(hit) =
                    matcher.press(&shortcuts.layout, &keymap, &Press { input: &shortcut.input, repeat: false, now, active: &active, outer })
                else {
                    continue;
                };
                let (app, instance) = with(ctx, |board| board.target_of(ctx, &hit.command));
                // A widget that only took the press as a move leaves it to the scoped commands around the focus.
                if app && shortcut.moved {
                    continue;
                }
                let resolved = resolve_in(ctx, &hit.command, instance);
                if !resolved.enabled {
                    continue;
                }
                // An app-scope command a modal layer's instance declares is delivered to that instance.
                let target = if app { resolved.instance } else { instance };
                let key = shortcut.input.physical.or(shortcut.input.typed);
                with(ctx, |board| {
                    if let Some(key) = key.filter(|_| board.registry.get(&hit.command.id).is_some_and(|def| def.kind == CommandKind::Momentary)) {
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
                });
            }
        }
    }
    with(ctx, |board| board.matcher = matcher);
    for fired in fired {
        queue(ctx, fired);
    }
}

/// The end hook: instances that stopped registering are gone, and so are the commands queued for them.
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
        board.focused.retain(|(viewport, _)| live.contains(viewport));
    });
}
