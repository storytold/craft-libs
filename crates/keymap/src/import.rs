//! Importing another application's shortcut set.
//!
//! The app's reader turns a file into a [`ForeignSet`]. [`plan`] matches its entries to the
//! keymap's commands without changing anything, and [`ImportPlan::apply`] applies the plan and
//! returns an [`ImportReport`]. Matching and the apply rules are ported from the DesignCraft
//! InDesign importer (#1180).

pub mod namematch;

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::chord::Trigger;
use crate::chord::foreign::{Flavor, ForeignError, ReadNote, read_foreign};
use crate::command::CommandRef;
use crate::keymap::{Bindable, Binding, Keymap, Kind, MAX_BINDINGS, Policy};
use crate::scope::Scope;

/// Most entries of a set read; the rest are ignored and counted in [`ImportReport::truncated`].
pub const MAX_IMPORT_ENTRIES: usize = 20_000;
/// Most keys read per entry.
pub const MAX_KEYS_PER_ENTRY: usize = 64;
/// Longest text field read, in chars; longer ones are cut.
pub const MAX_IMPORT_TEXT: usize = 1024;
/// Most path segments read per entry; the label (last segment) is always kept.
pub const MAX_PATH_SEGMENTS: usize = 32;
/// Most unmatched entries [`ImportOptions::suggest`] looks for commands for; the rest are counted
/// in [`ImportReport::truncated`]. Entries without keys get no suggestion and do not count.
pub const MAX_SUGGESTIONS: usize = 200;

/// A shortcut set as an app's reader found it. Build it with [`ForeignSet::new`].
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ForeignSet {
    pub name: Option<String>,
    pub entries: Vec<ForeignEntry>,
    /// Entries the reader dropped at its own limits; [`plan`] counts them in
    /// [`ImportReport::truncated`].
    pub truncated: usize,
}

/// One item of a foreign set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ForeignEntry {
    /// The top menu or product area as the source names it, without decoration such as a
    /// trailing " Menu".
    pub menu: Option<String>,
    /// Submenu segments and the item label; the label is last.
    pub path: Vec<String>,
    /// Empty when the source says the item has no shortcut.
    pub keys: Vec<ForeignKey>,
    pub line: Option<usize>,
    /// The item selects a tool.
    pub is_tool: bool,
    /// The entry belongs to a panel menu: its leading path segments name the panel, not a
    /// submenu of [`ForeignEntry::menu`], which is usually `None`. Sources write such items as
    /// `Character: All Caps` (InDesign's Panel Menus area, the panel-menu areas of Illustrator and
    /// Photoshop sets); the reader splits them into segments. Such an entry also matches a
    /// catalog path that ends with its path, and its last segment alone when its first segment
    /// is a segment of some catalog path. Without keys, an entry matched only these ways
    /// changes nothing ([`ImportReport::matched_without_change`]).
    pub panel_menu: bool,
}

impl ForeignSet {
    /// A set without a name.
    pub fn new(entries: Vec<ForeignEntry>) -> ForeignSet {
        ForeignSet { entries, ..ForeignSet::default() }
    }

    /// The set's name as the source gives it.
    pub fn with_name(mut self, name: impl Into<String>) -> ForeignSet {
        self.name = Some(name.into());
        self
    }

    /// Entries the reader dropped at its own limits ([`ForeignSet::truncated`]).
    pub fn with_truncated(mut self, truncated: usize) -> ForeignSet {
        self.truncated = truncated;
        self
    }
}

impl ForeignEntry {
    /// An entry without keys.
    pub fn new<S: Into<String>>(menu: Option<&str>, path: impl IntoIterator<Item = S>) -> ForeignEntry {
        ForeignEntry { menu: menu.map(str::to_string), path: path.into_iter().map(Into::into).collect(), ..ForeignEntry::default() }
    }

    /// Marks the entry as selecting a tool.
    pub fn tool(mut self) -> ForeignEntry {
        self.is_tool = true;
        self
    }

    /// Marks the entry as an item of a panel menu ([`ForeignEntry::panel_menu`]).
    pub fn in_panel_menu(mut self) -> ForeignEntry {
        self.panel_menu = true;
        self
    }

    /// Adds a key after the others.
    pub fn with_key(mut self, key: ForeignKey) -> ForeignEntry {
        self.keys.push(key);
        self
    }

    /// The entry's line in the source file.
    pub fn at_line(mut self, line: usize) -> ForeignEntry {
        self.line = Some(line);
        self
    }

    /// The source's own path, menu first, joined with ` > `: the `item` of report rows.
    pub fn item(&self) -> String {
        self.menu.iter().chain(&self.path).map(String::as_str).collect::<Vec<_>>().join(" > ")
    }

    /// The key of [`ImportOptions::remembered`] for this entry: the normalized menu and path
    /// joined by U+001F.
    pub fn remember_key(&self) -> String {
        self.menu.iter().chain(&self.path).map(|s| normalize_label(s)).collect::<Vec<_>>().join("\u{1f}")
    }
}

/// One shortcut of an entry.
#[derive(Clone, Debug, PartialEq)]
pub struct ForeignKey {
    /// The context the source gives (InDesign's `Default`, `Text`, …); `None` reads as a
    /// [`ContextRule::Home`] context.
    pub context: Option<String>,
    /// The key text as written, for reports.
    pub text: String,
    pub trigger: Result<Trigger, ForeignError>,
    /// What reading the text approximated, reported in [`ImportReport::approximated`].
    pub notes: Vec<ReadNote>,
}

impl ForeignKey {
    /// Reads `text` with [`read_foreign`].
    pub fn read(context: Option<&str>, text: &str, flavor: Flavor) -> ForeignKey {
        let (trigger, notes) = match read_foreign(text, flavor) {
            Ok((t, notes)) => (Ok(t), notes),
            Err(e) => (Err(e), Vec::new()),
        };
        ForeignKey { context: context.map(str::to_string), text: text.to_string(), trigger, notes }
    }
}

/// What keys in a source context become.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContextRule {
    /// Bindings in the command's home scope.
    Home,
    /// Bindings in this scope.
    Scope(Scope),
    /// Bindings in the home scope, only when the entry has no key in a [`ContextRule::Home`]
    /// context (DesignCraft reads InDesign's `Text` context this way).
    HomeWithoutHomeEntry,
    /// Ignored without a report row.
    Skip,
}

/// What an import starts from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Start {
    /// The keymap's bindings as they are.
    #[default]
    Current,
    /// Every command's defaults; other overrides are dropped.
    Defaults,
}

/// What an entry without keys does to its command.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoneDefined {
    /// The command loses its shortcuts, unless another entry matched to it has keys or the entry
    /// matched only through a panel-menu rule ([`NoChangeReason::PanelMenu`]).
    #[default]
    Clear,
    /// The command keeps its shortcuts.
    Keep,
}

/// How [`plan`] reads a set and how [`ImportPlan::apply`] applies it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImportOptions {
    /// Source contexts by name, compared case-insensitively. Keys in other contexts are reported
    /// as [`UnmatchedReason::Context`].
    pub contexts: Vec<(String, ContextRule)>,
    /// Earlier answers of the user, by [`ForeignEntry::remember_key`]. They are tried first.
    pub remembered: BTreeMap<String, CommandRef>,
    pub start: Start,
    pub none_defined: NoneDefined,
    /// Look for likely commands for unmatched entries ([`ImportReport::suggestions`]).
    pub suggest: bool,
}

/// One planned binding, with the entry it came from.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct PlannedBinding {
    pub binding: Binding,
    pub item: String,
    pub line: Option<usize>,
    /// The entry's position in the set: a later entry takes a chord from an earlier one.
    pub order: usize,
}

/// What the set says about one command, over every entry matched to it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Planned {
    pub command: CommandRef,
    /// The first entry matched to it, and its line.
    pub item: String,
    pub line: Option<usize>,
    pub bindings: Vec<PlannedBinding>,
    /// Every matched entry had no keys, so [`NoneDefined::Clear`] clears the command. One entry
    /// with keys, in any order, keeps an entry without keys from clearing them.
    pub none_defined: bool,
}

/// The result of [`plan`]: what would change, and the report rows known before applying.
///
/// Serializable for a review UI; it does not deserialize, so every plan applied comes from
/// [`plan`] and keeps its caps.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ImportPlan {
    /// In the order of their first entry.
    pub commands: Vec<Planned>,
    pub start: Start,
    pub none_defined: NoneDefined,
    /// `unmatched`, `unreadable`, `approximated`, `suggestions`, `matched_without_change` and the
    /// counts;
    /// [`ImportPlan::apply`] adds the rest.
    pub report: ImportReport,
}

/// A command whose bindings the import changed. Shortcuts are binding texts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Changed {
    pub command: CommandRef,
    pub label: String,
    /// The first entry matched to the command, and its line.
    pub item: String,
    pub line: Option<usize>,
    pub from: Vec<String>,
    pub to: Vec<String>,
}

/// A binding taken from a command outside the set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Cleared {
    pub command: CommandRef,
    pub label: String,
    pub shortcut: String,
}

/// Why a key of the set was not bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum SkipReason {
    /// A later entry of the set takes the chord.
    Taken,
    /// The system takes the chord.
    Reserved,
    /// The binding's scope ([`ContextRule::Scope`]) is not one the keymap knows
    /// ([`Keymap::knows_scope`]).
    UnknownScope,
}

/// A key of the set that was not bound.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Skipped {
    pub command: CommandRef,
    pub item: String,
    pub line: Option<usize>,
    pub shortcut: String,
    /// The command of the later entry that takes the chord ([`SkipReason::Taken`]).
    pub kept_by: Option<CommandRef>,
    pub reason: SkipReason,
}

/// A change [`ImportPlan::apply`] could not make, so the command keeps the bindings it had. The
/// overrides had no room for another changed command ([`AssignError::Full`]), or the keymap lacks
/// the command (a plan applied to a keymap it was not made from).
///
/// [`AssignError::Full`]: crate::AssignError::Full
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Failed {
    pub command: CommandRef,
    pub label: String,
    pub change: FailedChange,
    /// The set's shortcuts for [`FailedChange::Bind`]; empty for [`FailedChange::Clear`].
    pub shortcuts: Vec<String>,
}

/// What a [`Failed`] change was.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum FailedChange {
    /// Giving the command the set's bindings, or taking one of them from another command.
    Bind,
    /// Leaving the command without shortcuts: the set has it without keys and
    /// [`NoneDefined::Clear`] applies.
    Clear,
}

/// Why keys of an entry were not used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum UnmatchedReason {
    NoMatch,
    /// Several commands fit and no match step picked one.
    Ambiguous,
    /// The keys are in contexts [`ImportOptions::contexts`] does not list.
    Context,
    /// Only a command of the other kind fits: a tool entry naming a command, or the other way round.
    Tool,
}

/// An entry with keys that was not used, or keys of a matched entry in unlisted contexts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Unmatched {
    pub item: String,
    pub line: Option<usize>,
    /// The keys as written, `<context>: <keys>` joined with `, `.
    pub keys: String,
    pub reason: UnmatchedReason,
}

/// Keys of a matched entry that could not be read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Unreadable {
    pub item: String,
    pub line: Option<usize>,
    pub keys: String,
    /// Why the first of them could not be read.
    pub reason: ForeignError,
}

/// An entry without keys that matched a command and left its shortcuts alone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct MatchedWithoutChange {
    pub command: CommandRef,
    pub label: String,
    pub item: String,
    pub line: Option<usize>,
    pub reason: NoChangeReason,
}

/// Why a [`MatchedWithoutChange`] entry did not clear its command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum NoChangeReason {
    /// The entry matched only through a panel-menu rule (a catalog path ending with its
    /// segments, or its last segment alone). Such a match is close enough to bind the entry's
    /// keys, not to take a command's shortcuts away.
    PanelMenu,
}

/// Likely commands for an unmatched entry, best first. Never applied.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Suggestion {
    pub item: String,
    pub line: Option<usize>,
    pub candidates: Vec<CommandRef>,
}

/// Keys of a matched entry that were read approximately, such as a numeric keypad key bound as
/// the main key of the same name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Approximated {
    pub item: String,
    pub line: Option<usize>,
    /// The keys with this note, as written: `<context>: <keys>` joined with `, `.
    pub keys: String,
    pub note: ReadNote,
}

/// What an import did. Shortcuts are canonical binding texts; `item` is the source's path and
/// `line` the entry's [`ForeignEntry::line`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ImportReport {
    pub set: Option<String>,
    /// Commands of the set whose bindings changed.
    pub imported: usize,
    pub changed: Vec<Changed>,
    pub conflicts_cleared: Vec<Cleared>,
    pub skipped: Vec<Skipped>,
    #[serde(default)]
    pub failed: Vec<Failed>,
    /// Entries without keys that matched a command and changed nothing.
    #[serde(default)]
    pub matched_without_change: Vec<MatchedWithoutChange>,
    pub unmatched: Vec<Unmatched>,
    /// Entries that matched no command and have no keys, or keys only in
    /// [`ContextRule::Skip`] contexts.
    pub unmatched_without_shortcut: usize,
    pub unreadable: Vec<Unreadable>,
    pub approximated: Vec<Approximated>,
    pub suggestions: Vec<Suggestion>,
    /// Entries, keys, path segments, bindings and text fields cut at the caps, unmatched entries
    /// past [`MAX_SUGGESTIONS`] that got no suggestion, and [`ForeignSet::truncated`].
    pub truncated: usize,
}

/// The matching form of a label or menu name: lower case, no `…`, `...` or `&`, a colon read as
/// a space (so the alias `Preferences: General…` matches the path `Preferences > General...`),
/// single spaces.
pub fn normalize_label(s: &str) -> String {
    let s = s.replace('…', "").replace("...", "").replace('&', "").replace(':', " ");
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// A normalized name without a leading `show/hide `, `show ` or `hide ` when something remains:
/// sources name a toggle by what choosing it does now. Only toggle commands
/// ([`Bindable::is_toggle`]) are matched by this form.
fn toggle_name(s: &str) -> String {
    ["show/hide ", "show ", "hide "].iter().find_map(|p| s.strip_prefix(p).filter(|r| !r.is_empty())).unwrap_or(s).to_string()
}

/// Matches `set` to the keymap's commands. Pure: the keymap is not changed.
pub fn plan(set: &ForeignSet, keymap: &Keymap, options: &ImportOptions) -> ImportPlan {
    let mut cut = 0usize;
    let mut report = ImportReport { set: set.name.as_deref().map(|n| cap_text(n, &mut cut)), ..ImportReport::default() };
    cut = cut.saturating_add(set.entries.len().saturating_sub(MAX_IMPORT_ENTRIES)).saturating_add(set.truncated);
    let index = Index::new(keymap);
    let contexts: Vec<(String, &ContextRule)> = options.contexts.iter().map(|(n, r)| (n.to_lowercase(), r)).collect();
    let mut commands: Vec<Planned> = Vec::new();
    let mut planned_at: HashMap<usize, usize> = HashMap::new();
    let mut to_suggest: Vec<ToSuggest> = Vec::new();

    let rule_of = |k: &ForeignKey| -> Option<&ContextRule> {
        match &k.context {
            None => Some(&ContextRule::Home),
            Some(c) => {
                let c = c.to_lowercase();
                contexts.iter().find(|(n, _)| *n == c).map(|(_, r)| *r)
            }
        }
    };

    for (order, raw) in set.entries.iter().take(MAX_IMPORT_ENTRIES).enumerate() {
        let entry = capped(raw, &mut cut);
        let item = entry.item();
        let found = index.find(&entry, entry.is_tool, &options.remembered);
        let Found::One(i, by_panel) = found else {
            // Keys in skipped contexts would bind nothing, so they neither make a row nor count
            // toward the suggestion cap.
            let keys: Vec<&ForeignKey> = entry.keys.iter().filter(|k| rule_of(k) != Some(&ContextRule::Skip)).collect();
            if keys.is_empty() {
                report.unmatched_without_shortcut += 1;
                continue;
            }
            let reason = match found {
                Found::Many => UnmatchedReason::Ambiguous,
                _ if matches!(index.find(&entry, !entry.is_tool, &BTreeMap::new()), Found::One(..)) => UnmatchedReason::Tool,
                _ => UnmatchedReason::NoMatch,
            };
            if options.suggest && reason != UnmatchedReason::Tool {
                if to_suggest.len() < MAX_SUGGESTIONS {
                    let label = entry.path.last().cloned().unwrap_or_default();
                    to_suggest.push(ToSuggest { item: item.clone(), line: entry.line, label, tool: entry.is_tool });
                } else {
                    cut += 1;
                }
            }
            report.unmatched.push(Unmatched { item, line: entry.line, keys: keys_text(keys.into_iter()), reason });
            continue;
        };

        if by_panel && entry.keys.is_empty() {
            let (command, label) =
                keymap.catalog().get(i).map(|b| (b.command.clone(), b.label.clone())).unwrap_or_else(|| (CommandRef::new(""), String::new()));
            report.matched_without_change.push(MatchedWithoutChange { command, label, item, line: entry.line, reason: NoChangeReason::PanelMenu });
            continue;
        }

        let has_home = entry.keys.iter().any(|k| rule_of(k) == Some(&ContextRule::Home));
        let mut unlisted = Vec::new();
        let mut unreadable = Vec::new();
        let mut approximated: Vec<(ReadNote, Vec<&ForeignKey>)> = Vec::new();
        let mut bindings = Vec::new();
        for k in &entry.keys {
            let scope = match rule_of(k) {
                None => {
                    unlisted.push(k);
                    continue;
                }
                Some(ContextRule::Skip) => continue,
                Some(ContextRule::HomeWithoutHomeEntry) if has_home => continue,
                Some(ContextRule::Home | ContextRule::HomeWithoutHomeEntry) => None,
                Some(ContextRule::Scope(s)) => Some(s.clone()),
            };
            match &k.trigger {
                Ok(t) => bindings.push(Binding { trigger: *t, scope }),
                Err(e) => unreadable.push((k, e)),
            }
            for &note in &k.notes {
                match approximated.iter_mut().find(|(n, _)| *n == note) {
                    Some((_, keys)) => keys.push(k),
                    None => approximated.push((note, vec![k])),
                }
            }
        }
        let line = entry.line;
        if !unlisted.is_empty() {
            report.unmatched.push(Unmatched { item: item.clone(), line, keys: keys_text(unlisted.into_iter()), reason: UnmatchedReason::Context });
        }
        if let Some((_, e)) = unreadable.first() {
            let reason = (*e).clone();
            report.unreadable.push(Unreadable { item: item.clone(), line, keys: keys_text(unreadable.iter().map(|(k, _)| *k)), reason });
        }
        for (note, keys) in approximated {
            report.approximated.push(Approximated { item: item.clone(), line, keys: keys_text(keys.into_iter()), note });
        }

        let at = *planned_at.entry(i).or_insert_with(|| {
            let command = keymap.catalog().get(i).map(|b| b.command.clone()).unwrap_or_else(|| CommandRef::new(""));
            commands.push(Planned { command, item: item.clone(), line, bindings: Vec::new(), none_defined: true });
            commands.len() - 1
        });
        let Some(p) = commands.get_mut(at) else { continue };
        p.none_defined &= entry.keys.is_empty();
        for binding in bindings {
            if p.bindings.iter().any(|b| b.binding == binding) {
                continue;
            }
            if p.bindings.len() >= MAX_BINDINGS {
                cut += 1;
                continue;
            }
            p.bindings.push(PlannedBinding { binding, item: item.clone(), line, order });
        }
    }

    if !to_suggest.is_empty() {
        report.suggestions = suggest(keymap, &to_suggest);
    }
    report.truncated = cut;
    ImportPlan { commands, start: options.start, none_defined: options.none_defined, report }
}

impl ImportPlan {
    /// Applies the plan to `keymap`, which should be the one it was made from. The set wins: a
    /// chord it gives to a command is taken from other commands (except twins sharing it as a
    /// default). Within the set, a later entry takes a chord from an earlier one, as when the
    /// entries are applied in file order; the earlier entry's key is reported in
    /// [`ImportReport::skipped`].
    ///
    /// A change that finds the overrides full is left out and reported in
    /// [`ImportReport::failed`]; the others still apply, and a chord it would have taken from an
    /// earlier entry stays with that entry. That includes taking a chord from another command,
    /// or from a command whose own change failed: both commands keep the bindings they had. With
    /// [`Start::Defaults`] that cannot happen.
    pub fn apply(&self, keymap: &mut Keymap) -> ImportReport {
        let before: Vec<Vec<Binding>> = self.commands.iter().map(|p| keymap.bindings(&p.command)).collect();
        // Dropping every entry leaves room for all of them: each catalog command takes at most one
        // entry, and the catalog holds at most `MAX_COMMANDS`.
        if self.start == Start::Defaults {
            keymap.reset_all();
        }
        let start = keymap.clone();
        // Commands whose bindings found the overrides full after taking a chord from an earlier
        // entry. They are left out of the claims and the plan is applied again, so the earlier
        // entry keeps the chord, as when the entries are applied in file order.
        let mut dropped: HashMap<usize, Failed> = HashMap::new();
        'attempt: loop {
            let mut report = self.report.clone();
            let (accepted, displaced) = self.resolve(keymap, &dropped, &mut report);
            let home_of = |km: &Keymap, c: &CommandRef| km.bindable(c).map_or_else(Scope::app, |b| b.scope.clone());
            let label_of = |km: &Keymap, c: &CommandRef| km.bindable(c).map(|b| b.label.clone()).unwrap_or_default();
            let mut decided: HashSet<String> = HashSet::new();
            // Commands whose change failed keep the bindings they had, whatever later changes free.
            let mut failed_here: HashSet<String> = HashSet::new();
            for (c, (p, list)) in self.commands.iter().zip(&accepted).enumerate() {
                let failed =
                    |km: &Keymap, change, shortcuts| Failed { command: p.command.clone(), label: label_of(km, &p.command), change, shortcuts };
                if let Some(f) = dropped.get(&c) {
                    report.failed.push(f.clone());
                    failed_here.insert(p.command.key());
                    continue;
                }
                if !list.is_empty() {
                    let list = in_default_order(keymap, &p.command, list);
                    if keymap.set_bindings(&p.command, list.clone(), Policy::Keep).is_err() {
                        let home = home_of(keymap, &p.command);
                        let f = failed(keymap, FailedChange::Bind, list.iter().map(|b| b.text(&home)).collect());
                        if displaced.contains(&c) {
                            dropped.insert(c, f);
                            *keymap = start.clone();
                            continue 'attempt;
                        }
                        report.failed.push(f);
                        failed_here.insert(p.command.key());
                        continue;
                    }
                } else if p.none_defined && self.none_defined == NoneDefined::Clear {
                    if !keymap.clear(&p.command) {
                        report.failed.push(failed(keymap, FailedChange::Clear, Vec::new()));
                        failed_here.insert(p.command.key());
                        continue;
                    }
                } else {
                    continue;
                }
                decided.insert(p.command.key());
            }

            // A command that cannot take a chord from another command is dropped, so both keep
            // the bindings they had. That includes a command whose own change failed: freeing an
            // entry later must not take its chord away.
            let mut unbind_failed = false;
            for (c, (p, list)) in self.commands.iter().zip(&accepted).enumerate() {
                if list.is_empty() || !decided.contains(&p.command.key()) {
                    continue;
                }
                for h in keymap.clashing(&p.command, list) {
                    if decided.contains(&h.command.key()) {
                        continue;
                    }
                    let home = home_of(keymap, &h.command);
                    let label = label_of(keymap, &h.command);
                    let shortcut = h.binding.text(&home);
                    if !failed_here.contains(&h.command.key()) && keymap.remove(&h.command, &h.binding) {
                        report.conflicts_cleared.push(Cleared { command: h.command, label, shortcut });
                    } else {
                        let home = home_of(keymap, &p.command);
                        let list = in_default_order(keymap, &p.command, list);
                        let shortcuts = list.iter().map(|b| b.text(&home)).collect();
                        dropped.insert(
                            c,
                            Failed { command: p.command.clone(), label: label_of(keymap, &p.command), change: FailedChange::Bind, shortcuts },
                        );
                        unbind_failed = true;
                        break;
                    }
                }
            }
            if unbind_failed {
                *keymap = start.clone();
                continue 'attempt;
            }

            for (p, from) in self.commands.iter().zip(&before) {
                if !decided.contains(&p.command.key()) {
                    continue;
                }
                let to = keymap.bindings(&p.command);
                if to == *from {
                    continue;
                }
                let home = home_of(keymap, &p.command);
                let texts = |l: &[Binding]| l.iter().map(|b| b.text(&home)).collect::<Vec<_>>();
                report.changed.push(Changed {
                    command: p.command.clone(),
                    label: label_of(keymap, &p.command),
                    item: p.item.clone(),
                    line: p.line,
                    from: texts(from),
                    to: texts(&to),
                });
            }
            report.imported = report.changed.len();
            return report;
        }
    }

    /// Settles which command each claimed chord goes to, in entry order, leaving out the claims of
    /// `dropped` commands. Returns each command's accepted bindings and the commands that took a
    /// chord from an earlier entry; skipped keys go to `report`.
    fn resolve(&self, keymap: &Keymap, dropped: &HashMap<usize, Failed>, report: &mut ImportReport) -> (Vec<Vec<Binding>>, HashSet<usize>) {
        let home_of = |c: &CommandRef| keymap.bindable(c).map_or_else(Scope::app, |b| b.scope.clone());
        let mut claims: Vec<(usize, &PlannedBinding)> =
            self.commands.iter().enumerate().flat_map(|(c, p)| p.bindings.iter().map(move |b| (c, b))).collect();
        claims.sort_by_key(|(_, b)| b.order);
        let platform = keymap.platform();
        let mut taken: HashMap<Trigger, Vec<TakenGroup>> = HashMap::new();
        let mut accepted: Vec<Vec<&PlannedBinding>> = vec![Vec::new(); self.commands.len()];
        let mut displaced: HashSet<usize> = HashSet::new();
        for (c, pb) in claims {
            let Some(p) = self.commands.get(c) else { continue };
            let shortcut = || pb.binding.text(&home_of(&p.command));
            let skip =
                |reason| Skipped { command: p.command.clone(), item: pb.item.clone(), line: pb.line, shortcut: shortcut(), kept_by: None, reason };
            if keymap.is_reserved(&pb.binding.trigger.chord) {
                report.skipped.push(skip(SkipReason::Reserved));
                continue;
            }
            if pb.binding.scope.as_ref().is_some_and(|s| !keymap.knows_scope(s)) {
                report.skipped.push(skip(SkipReason::UnknownScope));
                continue;
            }
            if dropped.contains_key(&c) {
                continue;
            }
            let effective = pb.binding.trigger.effective(platform);
            let (scope, tools) = match keymap.bindable(&p.command) {
                Some(b) => (pb.binding.scope_or(&b.scope).clone(), if let Kind::Tool { group } = &b.kind { Some(group.clone()) } else { None }),
                None => (pb.binding.scope.clone().unwrap_or_else(Scope::app), None),
            };
            let groups = taken.entry(effective).or_default();
            let mut lost: Vec<(usize, &PlannedBinding)> = Vec::new();
            for g in groups.iter_mut().filter(|g| !(tools.is_some() && g.tools == tools) && keymap.scopes().relation(&g.scope, &scope) == Some(true))
            {
                g.holders.retain(|&(k, earlier)| {
                    let clash =
                        self.commands.get(k).is_some_and(|other| keymap.pair_clashes(&other.command, &earlier.binding, &p.command, &pb.binding));
                    if clash {
                        lost.push((k, earlier));
                    }
                    !clash
                });
            }
            if !lost.is_empty() {
                displaced.insert(c);
            }
            for (k, earlier) in lost {
                if let Some(list) = accepted.get_mut(k) {
                    list.retain(|b| !std::ptr::eq(*b, earlier));
                }
                let Some(other) = self.commands.get(k) else { continue };
                report.skipped.push(Skipped {
                    command: other.command.clone(),
                    item: earlier.item.clone(),
                    line: earlier.line,
                    shortcut: earlier.binding.text(&home_of(&other.command)),
                    kept_by: Some(p.command.clone()),
                    reason: SkipReason::Taken,
                });
            }
            match groups.iter_mut().find(|g| g.scope == scope && g.tools == tools) {
                Some(g) => g.holders.push((c, pb)),
                None => groups.push(TakenGroup { scope, tools, holders: vec![(c, pb)] }),
            }
            if let Some(list) = accepted.get_mut(c) {
                list.push(pb);
            }
        }
        let accepted = accepted.into_iter().map(|l| l.into_iter().map(|b| b.binding.clone()).collect()).collect();
        (accepted, displaced)
    }
}

/// Claims of one chord that [`ImportPlan::apply`] accepted, with one scope and tool group (as
/// [`Keymap::conflicts`] groups them), so a claim is compared only with groups it can clash with.
struct TakenGroup<'a> {
    scope: Scope,
    /// The tool group, for tools.
    tools: Option<Option<String>>,
    /// Plan command index and claim.
    holders: Vec<(usize, &'a PlannedBinding)>,
}

/// `list` in the order of the command's defaults when it holds the same bindings, so an import
/// equal to the defaults leaves no override.
fn in_default_order(keymap: &Keymap, command: &CommandRef, list: &[Binding]) -> Vec<Binding> {
    let Some(home) = keymap.bindable(command).map(|b| b.scope.clone()) else { return list.to_vec() };
    let key = |b: &Binding| (b.trigger, b.scope_or(&home).clone());
    let defaults = keymap.default_bindings(command);
    let mut a: Vec<_> = list.iter().map(key).collect();
    let mut d: Vec<_> = defaults.iter().map(key).collect();
    a.sort();
    d.sort();
    if a == d { defaults } else { list.to_vec() }
}

fn keys_text<'a>(keys: impl Iterator<Item = &'a ForeignKey>) -> String {
    keys.map(|k| match &k.context {
        Some(c) => format!("{c}: {}", k.text),
        None => k.text.clone(),
    })
    .collect::<Vec<_>>()
    .join(", ")
}

/// `s` cut to [`MAX_IMPORT_TEXT`] chars, counting a cut in `cut`.
fn cap_text(s: &str, cut: &mut usize) -> String {
    match s.char_indices().nth(MAX_IMPORT_TEXT) {
        Some((end, _)) => {
            *cut += 1;
            s.get(..end).unwrap_or_default().to_string()
        }
        None => s.to_string(),
    }
}

/// A copy of `e` within the caps.
fn capped(e: &ForeignEntry, cut: &mut usize) -> ForeignEntry {
    let mut path: Vec<String> = e.path.iter().take(MAX_PATH_SEGMENTS.saturating_sub(1)).map(|s| cap_text(s, cut)).collect();
    if e.path.len() > MAX_PATH_SEGMENTS {
        *cut += e.path.len() - MAX_PATH_SEGMENTS;
    }
    if let Some(last) = e.path.get(path.len()..).and_then(<[String]>::last) {
        path.push(cap_text(last, cut));
    }
    *cut += e.keys.len().saturating_sub(MAX_KEYS_PER_ENTRY);
    let keys = e
        .keys
        .iter()
        .take(MAX_KEYS_PER_ENTRY)
        .map(|k| ForeignKey {
            context: k.context.as_deref().map(|c| cap_text(c, cut)),
            text: cap_text(&k.text, cut),
            trigger: match &k.trigger {
                Ok(t) => Ok(*t),
                Err(ForeignError::UnknownKey(s)) => Err(ForeignError::UnknownKey(cap_text(s, cut))),
                Err(ForeignError::UnsupportedModifier(s)) => Err(ForeignError::UnsupportedModifier(cap_text(s, cut))),
                Err(other) => Err(other.clone()),
            },
            notes: k.notes.iter().fold(Vec::new(), |mut notes, n| {
                if !notes.contains(n) {
                    notes.push(*n);
                }
                notes
            }),
        })
        .collect();
    ForeignEntry { menu: e.menu.as_deref().map(|m| cap_text(m, cut)), path, keys, line: e.line, is_tool: e.is_tool, panel_menu: e.panel_menu }
}

enum Found {
    /// The command, and whether only a panel-menu step found it.
    One(usize, bool),
    Many,
    None,
}

/// Catalog names in one form, for the match steps.
#[derive(Default)]
struct Names {
    /// Normalized paths joined by U+001F.
    paths: HashMap<String, Vec<usize>>,
    /// The last two or more segments of paths, also with the label as the last one, joined by
    /// U+001F: for panel-menu entries.
    ends: HashMap<String, Vec<usize>>,
    /// (top menu, label, alias or path label) at any depth of that menu.
    in_menu: HashMap<(String, String), Vec<usize>>,
    /// Labels, aliases and path labels anywhere.
    anywhere: HashMap<String, Vec<usize>>,
}

fn add<K: std::hash::Hash + Eq>(map: &mut HashMap<K, Vec<usize>>, key: K, i: usize) {
    let list = map.entry(key).or_default();
    if list.last() != Some(&i) {
        list.push(i);
    }
}

impl Names {
    /// Adds command `i`, its label, aliases and the last segment of each path passed through `form`.
    fn add(&mut self, i: usize, b: &Bindable, form: fn(&str) -> String) {
        let label = form(&normalize_label(&b.label));
        let names: Vec<String> = std::iter::once(label.clone()).chain(b.aliases.iter().map(|a| form(&normalize_label(a)))).collect();
        for name in &names {
            add(&mut self.anywhere, name.clone(), i);
        }
        for path in b.paths.iter().chain(&b.import_paths) {
            let mut segs: Vec<String> = path.iter().map(|s| normalize_label(s)).collect();
            if let [_, .., last] = segs.as_mut_slice() {
                *last = form(last);
            }
            add(&mut self.paths, segs.join("\u{1f}"), i);
            let Some(top) = segs.first() else { continue };
            for name in &names {
                add(&mut self.in_menu, (top.clone(), name.clone()), i);
            }
            let [_, .., last] = segs.as_slice() else { continue };
            add(&mut self.in_menu, (top.clone(), last.clone()), i);
            add(&mut self.anywhere, last.clone(), i);
            let mut with_label = segs.clone();
            if let Some(l) = with_label.last_mut() {
                *l = label.clone();
            }
            for variant in [&segs, &with_label] {
                for start in 0..variant.len().saturating_sub(1) {
                    if let Some(end) = variant.get(start..) {
                        add(&mut self.ends, end.join("\u{1f}"), i);
                    }
                }
            }
        }
    }

    /// The lists the match steps look up for an entry's normalized menu and path, most specific
    /// first, each with whether it is a panel-menu step. `panel` is `Some(known)` for a panel-menu
    /// entry, `known` saying whether the catalog has its panel; only then does its last segment
    /// alone match.
    fn steps(&self, menu: Option<&String>, segs: &[String], panel: Option<bool>) -> Vec<(Option<&Vec<usize>>, bool)> {
        let Some(last) = segs.last() else { return Vec::new() };
        let full = menu.into_iter().chain(segs).map(String::as_str).collect::<Vec<_>>().join("\u{1f}");
        let mut steps = vec![(self.paths.get(&full), false)];
        let joined = (segs.len() > 1).then(|| segs.join(" "));
        if let Some(m) = menu {
            if let Some(j) = &joined {
                steps.push((self.in_menu.get(&(m.clone(), j.clone())), false));
            }
            steps.push((self.in_menu.get(&(m.clone(), last.clone())), false));
        }
        if panel.is_some() && segs.len() > 1 {
            steps.push((self.ends.get(&segs.join("\u{1f}")), true));
        }
        if let Some(j) = &joined {
            steps.push((self.anywhere.get(j), false));
        }
        if menu.is_some() || segs.len() == 1 {
            steps.push((self.anywhere.get(last), false));
        } else if panel == Some(true) {
            steps.push((self.anywhere.get(last), true));
        }
        steps
    }
}

/// The catalog by normalized path and label, for the match steps.
struct Index {
    tool: Vec<bool>,
    /// Binding keys, for remembered answers.
    keys: HashMap<String, usize>,
    names: Names,
    /// Toggle commands by their names without a leading Show/Hide ([`toggle_name`]).
    toggles: Names,
    /// Every normalized segment of every catalog path: the panels and menus the app has.
    segments: HashSet<String>,
}

impl Index {
    fn new(keymap: &Keymap) -> Index {
        let mut ix = Index { tool: Vec::new(), keys: HashMap::new(), names: Names::default(), toggles: Names::default(), segments: HashSet::new() };
        for (i, b) in keymap.catalog().iter().enumerate() {
            ix.tool.push(matches!(b.kind, Kind::Tool { .. }));
            ix.keys.insert(b.command.key(), i);
            ix.names.add(i, b, str::to_string);
            ix.segments.extend(b.paths.iter().chain(&b.import_paths).flatten().map(|s| normalize_label(s)));
            if b.is_toggle {
                ix.toggles.add(i, b, toggle_name);
            }
        }
        ix
    }

    fn pick(&self, list: Option<&Vec<usize>>, tool: bool) -> Found {
        let mut fits = list.into_iter().flatten().filter(|i| self.tool.get(**i) == Some(&tool));
        match (fits.next(), fits.next()) {
            (Some(i), None) => Found::One(*i, false),
            (Some(_), Some(_)) => Found::Many,
            _ => Found::None,
        }
    }

    /// The match steps in order; the first that finds exactly one command decides. The steps run
    /// on the names as written, then on toggle commands with the entry's label and theirs
    /// without a leading Show/Hide. A panel-menu entry's last segment alone matches only when
    /// its first segment, the panel, is a segment of some catalog path.
    fn find(&self, entry: &ForeignEntry, tool: bool, remembered: &BTreeMap<String, CommandRef>) -> Found {
        if let Some(c) = remembered.get(&entry.remember_key())
            && let Some(&i) = self.keys.get(&c.key())
            && self.tool.get(i) == Some(&tool)
        {
            return Found::One(i, false);
        }
        let segs: Vec<String> = entry.path.iter().map(|s| normalize_label(s)).collect();
        let mut toggle_segs = segs.clone();
        if let Some(last) = toggle_segs.last_mut() {
            *last = toggle_name(last);
        }
        let menu = entry.menu.as_deref().map(normalize_label);
        let panel = entry.panel_menu.then(|| segs.first().is_some_and(|p| self.segments.contains(p)));
        let steps = self.names.steps(menu.as_ref(), &segs, panel);
        let toggle_steps = self.toggles.steps(menu.as_ref(), &toggle_segs, panel);
        let mut many = false;
        for (list, by_panel) in steps.into_iter().chain(toggle_steps) {
            match self.pick(list, tool) {
                Found::One(i, _) => return Found::One(i, by_panel),
                Found::Many => many = true,
                Found::None => {}
            }
        }
        if many { Found::Many } else { Found::None }
    }
}

/// An unmatched entry to look for likely commands for.
struct ToSuggest {
    item: String,
    line: Option<usize>,
    label: String,
    tool: bool,
}

/// Fuzzy candidates for unmatched entries, among commands of the same kind.
fn suggest(keymap: &Keymap, entries: &[ToSuggest]) -> Vec<Suggestion> {
    let pool = |tool: bool| -> Vec<&Bindable> { keymap.catalog().iter().filter(|b| matches!(b.kind, Kind::Tool { .. }) == tool).collect() };
    let matcher = |pool: &[&Bindable]| {
        let candidates: Vec<namematch::Candidate> = pool
            .iter()
            .map(|b| {
                let labels = std::iter::once(&b.label)
                    .chain(&b.aliases)
                    .chain(b.paths.iter().chain(&b.import_paths).filter_map(|p| p.last()))
                    .cloned()
                    .collect();
                namematch::Candidate { id: b.command.id.clone(), labels }
            })
            .collect();
        namematch::Matcher::new(&candidates)
    };
    let pools = [pool(false), pool(true)];
    let mut matchers: [Option<namematch::Matcher>; 2] = [None, None];
    let mut out = Vec::new();
    for ToSuggest { item, line, label, tool } in entries {
        let k = usize::from(*tool);
        let (Some(pool), Some(slot)) = (pools.get(k), matchers.get_mut(k)) else { continue };
        let picks = match slot.get_or_insert_with(|| matcher(pool)).propose(label) {
            namematch::Proposal::Match(i, _) => vec![i],
            namematch::Proposal::Choice(v) => v,
        };
        let candidates: Vec<CommandRef> = picks.iter().filter_map(|i| pool.get(*i)).map(|b| b.command.clone()).collect();
        if !candidates.is_empty() {
            out.push(Suggestion { item: item.clone(), line: *line, candidates });
        }
    }
    out
}
