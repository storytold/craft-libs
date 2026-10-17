//! A command catalogue with default bindings, the user's overrides, dispatch and conflicts.

pub(crate) mod overrides;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::chord::{Chord, ParseError, Platform, Trigger};
use crate::command::{CommandRef, MAX_KEY};
use crate::scope::{Active, Scope, ScopeKind, Scopes};

pub use overrides::Overrides;

/// Most commands a catalogue or an [`Overrides`] holds.
pub const MAX_COMMANDS: usize = 4096;
/// Most bindings per command.
pub const MAX_BINDINGS: usize = 16;
/// Longest binding text, in bytes.
pub const MAX_BINDING_TEXT: usize = 128;
/// Most entries [`Keymap::conflicts`] returns.
pub const MAX_CONFLICTS: usize = 4096;

/// What a binding does when pressed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    #[default]
    Command,
    /// Selects a tool. Tools of one group share keys and cycle; tools without a group name form one
    /// group together.
    Tool { group: Option<String> },
    /// A tool held down for temporary use.
    Hold,
}

/// A trigger in a scope. `scope: None` is the command's home scope.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Binding {
    pub trigger: Trigger,
    pub scope: Option<Scope>,
}

/// Why binding text was refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BindingError {
    TooLong,
    /// The part before `": "` is not a scope name.
    Scope,
    Trigger(ParseError),
}

impl fmt::Display for BindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindingError::TooLong => write!(f, "binding text over {MAX_BINDING_TEXT} bytes"),
            BindingError::Scope => f.write_str("bad scope name"),
            BindingError::Trigger(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for BindingError {}

impl Binding {
    /// A binding in the command's home scope.
    pub fn new(trigger: impl Into<Trigger>) -> Binding {
        Binding { trigger: trigger.into(), scope: None }
    }

    pub fn in_scope(trigger: impl Into<Trigger>, scope: Scope) -> Binding {
        Binding { trigger: trigger.into(), scope: Some(scope) }
    }

    /// Reads `<trigger>` or `<Scope>: <trigger>`. The whole text is tried as a trigger first, so
    /// `Cmd+: Cmd+:` is the Colon chord pressed twice; otherwise it is split at the first `": "`.
    pub fn parse(text: &str) -> Result<Binding, BindingError> {
        if text.len() > MAX_BINDING_TEXT {
            return Err(BindingError::TooLong);
        }
        let whole = match Trigger::parse(text.trim()) {
            Ok(trigger) => return Ok(Binding::new(trigger)),
            Err(e) => e,
        };
        match text.split_once(": ") {
            Some((scope, trigger)) => {
                let scope = Scope::new(scope.trim()).ok_or(BindingError::Scope)?;
                let trigger = Trigger::parse(trigger.trim()).map_err(BindingError::Trigger)?;
                Ok(Binding::in_scope(trigger, scope))
            }
            None => Err(BindingError::Trigger(whole)),
        }
    }

    /// The binding text for a command whose home scope is `home`: the scope is written only when
    /// it differs.
    pub fn text(&self, home: &Scope) -> String {
        match &self.scope {
            Some(s) if s != home => format!("{s}: {}", self.trigger),
            _ => self.trigger.to_string(),
        }
    }

    /// The scope, `home` when unset.
    pub fn scope_or<'a>(&'a self, home: &'a Scope) -> &'a Scope {
        self.scope.as_ref().unwrap_or(home)
    }
}

impl fmt::Display for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.scope {
            Some(s) => write!(f, "{s}: {}", self.trigger),
            None => write!(f, "{}", self.trigger),
        }
    }
}

impl Serialize for Binding {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Binding {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Binding, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        Binding::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// A command the user can bind, as the app describes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bindable {
    pub command: CommandRef,
    pub label: String,
    /// Menu paths, the main one first; used by search and import.
    #[serde(default)]
    pub paths: Vec<Vec<String>>,
    /// Other names: labels in other UI languages, older names.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Home scope.
    pub scope: Scope,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default)]
    pub defaults: Vec<Binding>,
}

impl Bindable {
    /// A command in `scope` without default bindings.
    pub fn new(command: impl Into<CommandRef>, label: impl Into<String>, scope: Scope) -> Bindable {
        Bindable {
            command: command.into(),
            label: label.into(),
            paths: Vec::new(),
            aliases: Vec::new(),
            scope,
            kind: Kind::Command,
            defaults: Vec::new(),
        }
    }

    pub fn with_kind(mut self, kind: Kind) -> Bindable {
        self.kind = kind;
        self
    }

    pub fn with_default(mut self, binding: Binding) -> Bindable {
        self.defaults.push(binding);
        self
    }

    pub fn with_path(mut self, path: Vec<String>) -> Bindable {
        self.paths.push(path);
        self
    }

    pub fn with_alias(mut self, alias: impl Into<String>) -> Bindable {
        self.aliases.push(alias.into());
        self
    }
}

/// Which chords work while a text field has the keyboard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TypingRule {
    /// Plain chords (no cmd, ctrl or alt) go to the text; function keys still fire.
    #[default]
    PlainKeysWait,
    /// Only chords with the command modifier and function keys fire.
    CommandKeysOnly,
    /// Nothing fires.
    AllWait,
}

impl TypingRule {
    /// Whether `chord` may fire while typing.
    pub fn fires(self, chord: &Chord) -> bool {
        match self {
            TypingRule::PlainKeysWait => !chord.is_plain() || chord.is_function_key(),
            TypingRule::CommandKeysOnly => chord.has_command() || chord.is_function_key(),
            TypingRule::AllWait => false,
        }
    }
}

/// What a key press runs.
#[derive(Clone, Debug, PartialEq)]
pub enum Lookup {
    None,
    Run(CommandRef),
    /// Tools of one group share the chord: the app picks the next one (catalogue order).
    Cycle(Vec<CommandRef>),
}

/// What to do with other commands that hold a chord being assigned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Policy {
    /// Remove the chord from them.
    #[default]
    Steal,
    /// Assign anyway; both keep it.
    Keep,
    /// Do not assign.
    Refuse,
}

/// A command's binding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Holder {
    pub command: CommandRef,
    pub binding: Binding,
}

/// The result of an assignment.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Assigned {
    /// Bindings removed from other commands ([`Policy::Steal`]).
    pub taken_from: Vec<Holder>,
    /// Bindings of other commands that now clash ([`Policy::Keep`]).
    pub clashes: Vec<Holder>,
}

/// Why an assignment was refused.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AssignError {
    UnknownCommand,
    /// The system takes this chord.
    Reserved(Trigger),
    /// More than [`MAX_BINDINGS`] bindings.
    TooMany,
    /// Other commands hold the chord ([`Policy::Refuse`]).
    Taken(Vec<Holder>),
    /// A scope that is neither registered in [`Scopes`] nor used by the catalogue.
    UnknownScope(Scope),
    /// The change needs a new [`Overrides`] entry and it already holds [`MAX_COMMANDS`].
    Full,
}

impl fmt::Display for AssignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssignError::UnknownCommand => f.write_str("unknown command"),
            AssignError::Reserved(t) => write!(f, "{t} is reserved by the system"),
            AssignError::TooMany => write!(f, "more than {MAX_BINDINGS} bindings"),
            AssignError::Taken(h) => write!(f, "in use by {} other command(s)", h.len()),
            AssignError::UnknownScope(s) => write!(f, "unknown scope {s}"),
            AssignError::Full => write!(f, "more than {MAX_COMMANDS} changed commands"),
        }
    }
}

impl std::error::Error for AssignError {}

/// Why loading dropped part of an [`Overrides`] entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DropReason {
    KeyTooLong,
    /// The value is not a list of texts.
    NotAList,
    /// Items of the list that are not text.
    NotText,
    TextTooLong,
    /// Texts past [`MAX_BINDINGS`].
    TooManyBindings,
}

/// Something wrong in the stored overrides. Load reports these instead of failing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Problem {
    /// No command in the catalogue has this key; the entry is kept.
    UnknownCommand { key: String },
    /// A text that does not parse; the entry keeps it.
    BadBinding { key: String, text: String, error: BindingError },
    /// A reserved chord; not bound.
    Reserved { key: String, text: String },
    /// A scope that is neither registered in [`Scopes`] nor used by the catalogue; not bound.
    UnknownScope { key: String, text: String },
    /// Another entry names the same command; this one is ignored.
    DuplicateKey { key: String },
    /// Part of an entry dropped while loading.
    Dropped { key: String, reason: DropReason },
    /// Entries past [`MAX_COMMANDS`] dropped while loading.
    TooManyCommands { ignored: usize },
}

/// How two bindings on one chord relate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictKind {
    /// Only one of the two can run.
    Clash,
    /// Both work: the nearer scope runs where it applies, the other elsewhere.
    Shadow,
}

/// Two commands on one chord.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    /// The effective trigger on the keymap's platform.
    pub chord: Trigger,
    pub a: (CommandRef, Binding),
    pub b: (CommandRef, Binding),
    pub kind: ConflictKind,
    /// Both bindings are defaults.
    pub in_defaults: bool,
}

/// A binding with its scope resolved.
type Bound = (Trigger, Scope);

/// A command's bindings on one chord grouped by scope and tool group, for [`Keymap::conflicts`].
struct Group<'a> {
    scope: &'a Scope,
    /// The tool group for tools, `None` for other commands.
    tools: Option<&'a Option<String>>,
    /// (position in the chord's list, command, binding).
    items: Vec<(usize, usize, &'a Bound)>,
}

/// Two holders of one chord: their positions in the chord's list, whether they clash, and each
/// one's command and binding.
type Pair<'a> = (usize, usize, bool, (usize, &'a Bound), (usize, &'a Bound));

/// The catalogue, the scope registry, the platform and the user's overrides.
#[derive(Clone, Debug)]
pub struct Keymap {
    catalogue: Vec<Bindable>,
    keys: Vec<String>,
    index: HashMap<String, usize>,
    defaults: Vec<Vec<Bound>>,
    current: Vec<Vec<Bound>>,
    /// Effective trigger → (command, position in its `current` list), kept in step with `current`.
    by_trigger: HashMap<Trigger, BTreeSet<(usize, usize)>>,
    /// The overrides entry each command reads, when it has one.
    stored: Vec<Option<String>>,
    /// Home and default scopes of the catalogue.
    catalogue_scopes: HashSet<Scope>,
    scopes: Scopes,
    platform: Platform,
    typing: TypingRule,
    reserved: Vec<Chord>,
    overrides: Overrides,
}

impl Keymap {
    /// A keymap with default bindings. Entries past [`MAX_COMMANDS`], with a key over
    /// [`MAX_KEY`] bytes, or repeating an earlier key are left out.
    pub fn new(catalogue: Vec<Bindable>, scopes: Scopes, platform: Platform) -> Keymap {
        let mut km = Keymap {
            catalogue: Vec::new(),
            keys: Vec::new(),
            index: HashMap::new(),
            defaults: Vec::new(),
            current: Vec::new(),
            by_trigger: HashMap::new(),
            stored: Vec::new(),
            catalogue_scopes: HashSet::new(),
            scopes,
            platform,
            typing: TypingRule::default(),
            reserved: Vec::new(),
            overrides: Overrides::new(),
        };
        for b in catalogue {
            let key = b.command.key();
            if km.catalogue.len() >= MAX_COMMANDS || key.len() > MAX_KEY || km.index.contains_key(&key) {
                continue;
            }
            let defaults = km.normalize(b.defaults.iter().cloned(), &b.scope);
            km.catalogue_scopes.insert(b.scope.clone());
            km.catalogue_scopes.extend(defaults.iter().map(|(_, s)| s.clone()));
            km.index.insert(key.clone(), km.catalogue.len());
            km.keys.push(key);
            km.current.push(defaults.clone());
            km.defaults.push(defaults);
            km.stored.push(None);
            km.catalogue.push(b);
        }
        km.rebuild_index();
        km
    }

    pub fn with_typing_rule(mut self, rule: TypingRule) -> Keymap {
        self.typing = rule;
        self
    }

    /// Chords that cannot be assigned, e.g. [`reserved::system`](crate::reserved::system).
    pub fn with_reserved(mut self, chords: Vec<Chord>) -> Keymap {
        self.reserved = chords;
        self.reload();
        self
    }

    pub fn with_overrides(mut self, overrides: Overrides) -> Keymap {
        self.set_overrides(overrides);
        self
    }

    pub fn set_overrides(&mut self, overrides: Overrides) {
        self.overrides = overrides;
        self.reload();
    }

    pub fn overrides(&self) -> &Overrides {
        &self.overrides
    }

    pub fn platform(&self) -> Platform {
        self.platform
    }

    pub fn scopes(&self) -> &Scopes {
        &self.scopes
    }

    pub fn typing_rule(&self) -> TypingRule {
        self.typing
    }

    pub fn catalogue(&self) -> &[Bindable] {
        &self.catalogue
    }

    pub fn bindable(&self, command: &CommandRef) -> Option<&Bindable> {
        self.find(command).map(|i| &self.catalogue[i])
    }

    /// The effective bindings of a command; empty for an unknown one.
    pub fn bindings(&self, command: &CommandRef) -> Vec<Binding> {
        self.find(command).map(|i| self.current[i].iter().map(|b| self.public(i, b)).collect()).unwrap_or_default()
    }

    /// The default bindings of a command; empty for an unknown one.
    pub fn default_bindings(&self, command: &CommandRef) -> Vec<Binding> {
        self.find(command).map(|i| self.defaults[i].iter().map(|b| self.public(i, b)).collect()).unwrap_or_default()
    }

    pub fn is_reserved(&self, chord: &Chord) -> bool {
        let c = chord.effective(self.platform);
        self.reserved.iter().any(|r| r.effective(self.platform) == c)
    }

    /// Whether bindings may use `scope`: the application, a scope registered in [`Scopes`], or a
    /// home or default scope of the catalogue.
    pub fn knows_scope(&self, scope: &Scope) -> bool {
        scope.is_app() || self.scopes.knows(scope) || self.catalogue_scopes.contains(scope)
    }

    /// The command `chord` (pressed twice when `double`) runs with `active` focus.
    pub fn lookup(&self, chord: &Chord, double: bool, active: &Active) -> Lookup {
        let want = Trigger { chord: chord.effective(self.platform), double };
        if active.typing && !self.typing.fires(&want.chord) {
            return Lookup::None;
        }
        let Some(holders) = self.by_trigger.get(&want) else { return Lookup::None };
        // Each command's nearest rank; `holders` is in catalogue order.
        let mut ranks: Vec<(usize, u8)> = Vec::new();
        for &(i, k) in holders {
            let Some(r) = self.at(i, k).and_then(|(_, s)| self.scopes.rank(s, active)) else { continue };
            match ranks.last_mut() {
                Some((j, best)) if *j == i => *best = (*best).min(r),
                _ => ranks.push((i, r)),
            }
        }
        let Some(best) = ranks.iter().map(|(_, r)| *r).min() else { return Lookup::None };
        let winners: Vec<usize> = ranks.iter().filter(|(_, r)| *r == best).map(|(i, _)| *i).collect();
        match winners.as_slice() {
            [] => Lookup::None,
            [i] => Lookup::Run(self.catalogue[*i].command.clone()),
            [first, rest @ ..] => {
                if rest.iter().all(|j| self.same_group(*first, *j)) {
                    Lookup::Cycle(winners.iter().map(|i| self.catalogue[*i].command.clone()).collect())
                } else {
                    Lookup::Run(self.catalogue[*first].command.clone())
                }
            }
        }
    }

    /// Adds a binding to a command.
    pub fn assign(&mut self, command: &CommandRef, binding: Binding, policy: Policy) -> Result<Assigned, AssignError> {
        let i = self.find(command).ok_or(AssignError::UnknownCommand)?;
        let new = self.bound(i, &binding);
        if self.current[i].iter().any(|b| self.same(b, &new)) {
            return Ok(Assigned::default());
        }
        let mut list = self.current[i].clone();
        list.push(new.clone());
        self.apply(i, list, &[new], policy)
    }

    /// Replaces all bindings of a command.
    pub fn set_bindings(&mut self, command: &CommandRef, bindings: Vec<Binding>, policy: Policy) -> Result<Assigned, AssignError> {
        let i = self.find(command).ok_or(AssignError::UnknownCommand)?;
        if bindings.len() > MAX_BINDINGS {
            return Err(AssignError::TooMany);
        }
        let home = self.catalogue[i].scope.clone();
        let list = self.normalize(bindings, &home);
        let new: Vec<Bound> = list.iter().filter(|b| !self.current[i].iter().any(|c| self.same(c, b))).cloned().collect();
        self.apply(i, list, &new, policy)
    }

    /// Removes one binding (matched by effective trigger and scope). `false` when the command does
    /// not have it, or when the change needs a new [`Overrides`] entry and the overrides are full
    /// ([`AssignError::Full`]); nothing changes then.
    pub fn remove(&mut self, command: &CommandRef, binding: &Binding) -> bool {
        let Some(i) = self.find(command) else { return false };
        let gone = self.bound(i, binding);
        let before = self.current[i].len();
        let list: Vec<Bound> = self.current[i].iter().filter(|b| !self.same(b, &gone)).cloned().collect();
        list.len() != before && self.commit(vec![(i, list)]).is_ok()
    }

    /// Leaves a command without shortcuts, in every scope. `false` for an unknown command, or when
    /// the overrides are full as for [`Keymap::remove`].
    pub fn clear(&mut self, command: &CommandRef) -> bool {
        let Some(i) = self.find(command) else { return false };
        self.commit(vec![(i, Vec::new())]).is_ok()
    }

    /// Gives a command its default bindings back.
    pub fn reset(&mut self, command: &CommandRef) -> bool {
        let Some(i) = self.find(command) else { return false };
        let defaults = self.defaults[i].clone();
        // Back to the defaults removes the entry, so this never needs room.
        self.commit(vec![(i, defaults)]).is_ok()
    }

    /// Every command gets its defaults; unknown entries go too.
    pub fn reset_all(&mut self) {
        self.overrides.clear();
        self.reload();
    }

    /// Pairs of commands on one chord, at most [`MAX_CONFLICTS`], in chord order, then in
    /// catalogue order. Tools of one group and bindings of panels that are never active together
    /// are left out; with panel fallthrough two unrelated panels on one chord tie in a third panel,
    /// which is reported as a [`ConflictKind::Shadow`].
    ///
    /// When the cap cuts a chord's pairs, the pairs kept are the first found comparing the
    /// chord's holders grouped by scope and tool group, group by group in the order each group
    /// first appears in the catalogue; the kept pairs are still returned in chord order, then
    /// catalogue order.
    pub fn conflicts(&self) -> Vec<Conflict> {
        let mut chords: Vec<(&Trigger, &BTreeSet<(usize, usize)>)> = self.by_trigger.iter().filter(|(_, h)| h.len() > 1).collect();
        chords.sort_by_key(|(t, _)| **t);
        let mut out = Vec::new();
        for (chord, holders) in chords {
            if !self.chord_conflicts(*chord, holders, &mut out) {
                break;
            }
        }
        out
    }

    /// What is wrong with the stored overrides: load notes, unknown commands, texts that do not
    /// parse, reserved chords, unknown scopes, duplicate entries.
    pub fn problems(&self) -> Vec<Problem> {
        let mut out = self.overrides.load_notes().to_vec();
        for (key, texts) in self.overrides.iter() {
            let Some(i) = self.resolve_key(key) else {
                out.push(Problem::UnknownCommand { key: key.to_string() });
                continue;
            };
            if self.stored[i].as_deref() != Some(key) {
                out.push(Problem::DuplicateKey { key: key.to_string() });
                continue;
            }
            let home = &self.catalogue[i].scope;
            for text in texts {
                let (key, text) = (key.to_string(), text.clone());
                match Binding::parse(&text) {
                    Err(error) => out.push(Problem::BadBinding { key, text, error }),
                    Ok(b) if self.is_reserved(&b.trigger.chord) => out.push(Problem::Reserved { key, text }),
                    Ok(b) if !self.usable_scope(b.scope_or(home), home) => out.push(Problem::UnknownScope { key, text }),
                    Ok(_) => {}
                }
            }
        }
        out
    }

    /// Bindings of other commands that only one of them and `bindings` on `command` could run,
    /// leaving out tools of the same group and twins (as [`Policy::Steal`] does).
    pub(crate) fn clashing(&self, command: &CommandRef, bindings: &[Binding]) -> Vec<Holder> {
        let Some(i) = self.find(command) else { return Vec::new() };
        let new: Vec<Bound> = bindings.iter().map(|b| self.bound(i, b)).collect();
        self.clashes(i, &new).into_iter().map(|(j, b)| Holder { command: self.catalogue[j].command.clone(), binding: self.public(j, &b) }).collect()
    }

    /// Whether binding `x` of command `a` and binding `y` of another command `b` clash, by the
    /// rules of [`Keymap::clashing`].
    pub(crate) fn pair_clashes(&self, a: &CommandRef, x: &Binding, b: &CommandRef, y: &Binding) -> bool {
        let (Some(i), Some(j)) = (self.find(a), self.find(b)) else { return false };
        if i == j || self.same_group(i, j) {
            return false;
        }
        let (bx, by) = (self.bound(i, x), self.bound(j, y));
        let t = bx.0.effective(self.platform);
        t == by.0.effective(self.platform) && !self.twins(i, j, t) && self.scopes.relation(&bx.1, &by.1) == Some(true)
    }

    fn find(&self, command: &CommandRef) -> Option<usize> {
        self.index.get(&command.key()).copied()
    }

    fn resolve_key(&self, key: &str) -> Option<usize> {
        self.index.get(key).or_else(|| CommandRef::parse_key(key).and_then(|c| self.index.get(&c.key()))).copied()
    }

    fn at(&self, i: usize, k: usize) -> Option<&Bound> {
        self.current.get(i).and_then(|l| l.get(k))
    }

    fn bound(&self, i: usize, binding: &Binding) -> Bound {
        (binding.trigger, binding.scope_or(&self.catalogue[i].scope).clone())
    }

    fn public(&self, i: usize, (trigger, scope): &Bound) -> Binding {
        let scope = (*scope != self.catalogue[i].scope).then(|| scope.clone());
        Binding { trigger: *trigger, scope }
    }

    fn same(&self, a: &Bound, b: &Bound) -> bool {
        a.1 == b.1 && a.0.effective(self.platform) == b.0.effective(self.platform)
    }

    /// Whether a stored binding of a command with home scope `home` may use `scope`.
    fn usable_scope(&self, scope: &Scope, home: &Scope) -> bool {
        scope == home || self.knows_scope(scope)
    }

    /// Resolves scopes, drops repeats and caps the count.
    fn normalize(&self, bindings: impl IntoIterator<Item = Binding>, home: &Scope) -> Vec<Bound> {
        let mut out: Vec<Bound> = Vec::new();
        for b in bindings {
            let bound = (b.trigger, b.scope_or(home).clone());
            if out.len() < MAX_BINDINGS && !out.contains(&bound) {
                out.push(bound);
            }
        }
        out
    }

    fn same_group(&self, i: usize, j: usize) -> bool {
        match (&self.catalogue[i].kind, &self.catalogue[j].kind) {
            (Kind::Tool { group: a }, Kind::Tool { group: b }) => a == b,
            _ => false,
        }
    }

    /// Both commands have `trigger` among their defaults: they share it by design.
    fn twins(&self, i: usize, j: usize, trigger: Trigger) -> bool {
        let has = |k: usize| self.defaults[k].iter().any(|(t, _)| t.effective(self.platform) == trigger);
        has(i) && has(j)
    }

    /// Bindings of other commands that only one of `new` and them could run.
    fn clashes(&self, i: usize, new: &[Bound]) -> Vec<(usize, Bound)> {
        let mut out = Vec::new();
        let mut seen: HashSet<(usize, usize)> = HashSet::new();
        for (t, s) in new {
            let t = t.effective(self.platform);
            let Some(holders) = self.by_trigger.get(&t) else { continue };
            for &(j, k) in holders {
                if j == i || self.same_group(i, j) || self.twins(i, j, t) {
                    continue;
                }
                let Some(b) = self.at(j, k) else { continue };
                if self.scopes.relation(s, &b.1) == Some(true) && seen.insert((j, k)) {
                    out.push((j, b.clone()));
                }
            }
        }
        out
    }

    /// How two scopes on one chord relate for [`Keymap::conflicts`]: [`Scopes::relation`], and with
    /// panel fallthrough two unrelated panels shadow (they tie in any third panel).
    fn conflict_relation(&self, a: &Scope, b: &Scope) -> Option<bool> {
        self.scopes.relation(a, b).or_else(|| {
            let panels = self.scopes.kind(a) == ScopeKind::Panel && self.scopes.kind(b) == ScopeKind::Panel;
            (panels && self.scopes.panel_fallthrough()).then_some(false)
        })
    }

    /// Adds the conflicts on one chord to `out`; `false` once `out` is full.
    ///
    /// The holders are grouped by scope and tool group, and only groups that relate are compared,
    /// so pairs that are left out (unrelated panels, tools of one group) cost nothing per pair.
    fn chord_conflicts(&self, chord: Trigger, holders: &BTreeSet<(usize, usize)>, out: &mut Vec<Conflict>) -> bool {
        let mut groups: Vec<Group> = Vec::new();
        let mut group_of: HashMap<(&Scope, Option<&Option<String>>), usize> = HashMap::new();
        for (pos, &(i, k)) in holders.iter().enumerate() {
            let Some(b) = self.at(i, k) else { continue };
            let tools = match &self.catalogue[i].kind {
                Kind::Tool { group } => Some(group),
                _ => None,
            };
            let g = *group_of.entry((&b.1, tools)).or_insert_with(|| {
                groups.push(Group { scope: &b.1, tools, items: Vec::new() });
                groups.len() - 1
            });
            if let Some(group) = groups.get_mut(g) {
                group.items.push((pos, i, b));
            }
        }
        let room = MAX_CONFLICTS.saturating_sub(out.len());
        let mut found: Vec<Pair> = Vec::new();
        'groups: for (n, g) in groups.iter().enumerate() {
            for (m, h) in groups.iter().enumerate().skip(n) {
                if g.tools.is_some() && g.tools == h.tools {
                    continue;
                }
                let Some(clash) = self.conflict_relation(g.scope, h.scope) else { continue };
                let same = m == n;
                for (x, &(pa, i, a)) in g.items.iter().enumerate() {
                    let rest = if same { g.items.get(x + 1..).unwrap_or_default() } else { &h.items };
                    for &(pb, j, b) in rest {
                        if i == j {
                            continue;
                        }
                        if found.len() >= room {
                            break 'groups;
                        }
                        let pair = if pa < pb { (pa, pb, clash, (i, a), (j, b)) } else { (pb, pa, clash, (j, b), (i, a)) };
                        found.push(pair);
                    }
                }
            }
        }
        found.sort_by_key(|(pa, pb, ..)| (*pa, *pb));
        for (_, _, clash, (i, a), (j, b)) in found {
            out.push(Conflict {
                chord,
                a: (self.catalogue[i].command.clone(), self.public(i, a)),
                b: (self.catalogue[j].command.clone(), self.public(j, b)),
                kind: if clash { ConflictKind::Clash } else { ConflictKind::Shadow },
                in_defaults: self.defaults[i].contains(a) && self.defaults[j].contains(b),
            });
        }
        out.len() < MAX_CONFLICTS
    }

    fn apply(&mut self, i: usize, list: Vec<Bound>, new: &[Bound], policy: Policy) -> Result<Assigned, AssignError> {
        if list.len() > MAX_BINDINGS {
            return Err(AssignError::TooMany);
        }
        if let Some((t, _)) = new.iter().find(|(t, _)| self.is_reserved(&t.chord)) {
            return Err(AssignError::Reserved(*t));
        }
        let home = &self.catalogue[i].scope;
        if let Some((_, s)) = new.iter().find(|(_, s)| !self.usable_scope(s, home)) {
            return Err(AssignError::UnknownScope(s.clone()));
        }
        let clashes = self.clashes(i, new);
        let holders: Vec<Holder> =
            clashes.iter().map(|(j, b)| Holder { command: self.catalogue[*j].command.clone(), binding: self.public(*j, b) }).collect();
        let mut done = Assigned::default();
        let mut writes: Vec<(usize, Vec<Bound>)> = Vec::new();
        match policy {
            Policy::Refuse if !holders.is_empty() => return Err(AssignError::Taken(holders)),
            Policy::Refuse => {}
            Policy::Keep => done.clashes = holders,
            Policy::Steal => {
                let mut victims: Vec<usize> = Vec::new();
                let mut gone: HashMap<usize, Vec<&Bound>> = HashMap::new();
                for (j, b) in &clashes {
                    let list = gone.entry(*j).or_default();
                    if list.is_empty() {
                        victims.push(*j);
                    }
                    list.push(b);
                }
                for j in victims {
                    let gone = gone.get(&j).map(Vec::as_slice).unwrap_or_default();
                    let rest: Vec<Bound> = self.current[j].iter().filter(|o| !gone.contains(o)).cloned().collect();
                    writes.push((j, rest));
                }
                done.taken_from = holders;
            }
        }
        writes.push((i, list));
        self.commit(writes)?;
        Ok(done)
    }

    /// Stores several commands' bindings, or none of them when the overrides lack room for the
    /// new entries.
    fn commit(&mut self, writes: Vec<(usize, Vec<Bound>)>) -> Result<(), AssignError> {
        let needed = writes.iter().filter(|(i, list)| self.stored[*i].is_none() && *list != self.defaults[*i]).count();
        if self.overrides.len() + needed > MAX_COMMANDS {
            return Err(AssignError::Full);
        }
        for (i, list) in writes {
            self.store(i, list);
        }
        Ok(())
    }

    /// Writes a command's bindings to the overrides, dropping the entry when they equal the
    /// defaults. Callers check for room first ([`Keymap::commit`]); texts always fit, since a scope
    /// name, `": "` and the longest double press stay under [`MAX_BINDING_TEXT`].
    fn store(&mut self, i: usize, list: Vec<Bound>) {
        if let Some(old) = self.stored[i].take() {
            self.overrides.remove(&old);
        }
        if list != self.defaults[i] {
            let home = &self.catalogue[i].scope;
            let texts = list.iter().map(|b| self.public(i, b).text(home)).collect();
            if self.overrides.insert(self.keys[i].clone(), texts) {
                self.stored[i] = Some(self.keys[i].clone());
            }
        }
        self.set_current(i, list);
    }

    fn set_current(&mut self, i: usize, list: Vec<Bound>) {
        self.unindex(i);
        if let Some(c) = self.current.get_mut(i) {
            *c = list;
        }
        self.index_command(i);
    }

    fn index_command(&mut self, i: usize) {
        let Some(list) = self.current.get(i) else { return };
        for (k, (t, _)) in list.iter().enumerate() {
            self.by_trigger.entry(t.effective(self.platform)).or_default().insert((i, k));
        }
    }

    fn unindex(&mut self, i: usize) {
        let Some(list) = self.current.get(i) else { return };
        for (k, (t, _)) in list.iter().enumerate() {
            let t = t.effective(self.platform);
            if let Some(holders) = self.by_trigger.get_mut(&t) {
                holders.remove(&(i, k));
                if holders.is_empty() {
                    self.by_trigger.remove(&t);
                }
            }
        }
    }

    fn rebuild_index(&mut self) {
        self.by_trigger.clear();
        for i in 0..self.current.len() {
            self.index_command(i);
        }
    }

    /// Recomputes every command's bindings from the defaults and the overrides.
    fn reload(&mut self) {
        self.stored.iter_mut().for_each(|s| *s = None);
        self.current.clone_from(&self.defaults);
        // Exact keys first, so a canonical entry wins over a hand-written variant of it.
        let mut entries: Vec<(&str, &[String])> = self.overrides.iter().collect();
        entries.sort_by_key(|(k, _)| !self.index.contains_key(*k));
        let mut found: Vec<(usize, String, Vec<Bound>)> = Vec::new();
        let mut taken: HashSet<usize> = HashSet::new();
        for (key, texts) in entries {
            let Some(i) = self.resolve_key(key) else { continue };
            if !taken.insert(i) {
                continue;
            }
            let home = &self.catalogue[i].scope;
            let usable = texts
                .iter()
                .filter_map(|t| Binding::parse(t).ok())
                .filter(|b| !self.is_reserved(&b.trigger.chord) && self.usable_scope(b.scope_or(home), home));
            let list = self.normalize(usable, home);
            // An entry none of whose texts is usable here (a key name from a newer version, a scope
            // this app lacks, a chord this platform reserves) keeps the defaults; `[]` means no
            // shortcut.
            let list = if list.is_empty() && !texts.is_empty() { self.defaults[i].clone() } else { list };
            found.push((i, key.to_string(), list));
        }
        for (i, key, list) in found {
            self.stored[i] = Some(key);
            self.current[i] = list;
        }
        self.rebuild_index();
    }
}
