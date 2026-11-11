//! The table's model: areas (Show), filters, search, group headers, sorting and the cached list of
//! lines the table draws.

use std::collections::{BTreeSet, HashMap, HashSet};

use craft_keymap::chord::{Platform, Trigger};
use craft_keymap::command::CommandRef;
use craft_keymap::keymap::{Binding, CommandDef, CommandKind, ConflictKind, Keymap, Overrides};

use crate::shortcuts::Options;
use crate::shortcuts::words::{Say, around_chord};

/// One entry of the Show menu.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum AreaId {
    All,
    /// A top menu, `paths[0][0]`.
    Menu(String),
    Tools,
    /// A panel scope, by name.
    Panel(String),
    /// Commands with no path that are not tools.
    Other,
    /// An app area from [`Options::areas`], by id.
    App(&'static str),
    /// [`Options::fixed`].
    Fixed,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AreaEntry {
    pub id: AreaId,
    pub label: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum Filter {
    #[default]
    All,
    WithShortcut,
    /// An overrides entry exists for the command.
    Changed,
    /// The command is in a [`ConflictKind::Clash`].
    Conflicts,
}

impl Filter {
    pub(crate) const ALL: [Filter; 4] = [Filter::All, Filter::WithShortcut, Filter::Changed, Filter::Conflicts];

    pub(crate) fn label(self, text: &Say) -> String {
        let words = text.words();
        match self {
            Filter::All => &words.all,
            Filter::WithShortcut => &words.with_shortcut,
            Filter::Changed => &words.changed,
            Filter::Conflicts => &words.conflicts,
        }
        .as_str()
        .to_owned()
    }
}

/// A binding as a row shows it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Chip {
    pub binding: Binding,
    /// Trigger text, plus ` (Scope)` for a scope other than the command's home scope (display order).
    pub text: String,
    /// Not one of the command's defaults.
    pub changed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    /// Position in the catalog.
    pub index: usize,
    pub command: CommandRef,
    /// [`CommandRef::key`], for automation ids and the overrides.
    pub key: String,
    /// The translated label.
    pub label: String,
    /// Translated path parts between the group header and the label, shown dim before it.
    pub prefix: Vec<String>,
    pub chips: Vec<Chip>,
    /// Override texts that do not parse, as written.
    pub unparsed: Vec<String>,
    /// An overrides entry exists.
    pub changed: bool,
    /// A tool sharing a key with another tool of its group.
    pub cycles: bool,
    /// In a clash.
    pub conflict: bool,
}

/// A read-only row from [`Options::fixed`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FixedLine {
    pub label: String,
    pub keys: String,
    /// The label of a catalog command now bound to the row's chord: drawn struck through.
    pub now_used_by: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Line {
    Header { name: String, collapsed: bool },
    Row(Row),
    Fixed(FixedLine),
}

/// The lines for one query, area, filter and set of overrides.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Built {
    pub lines: Vec<Line>,
    /// Command and fixed rows shown (headers not counted); 0 shows "No matching commands".
    pub matches: usize,
    /// Clash pairs in the whole keymap.
    pub clashes: usize,
    /// Shadow pairs in the whole keymap, shown as an information line.
    pub shadows: usize,
}

impl Built {
    /// The line index of a command's row.
    pub(crate) fn position(&self, command: &CommandRef) -> Option<usize> {
        self.lines.iter().position(|l| matches!(l, Line::Row(r) if r.command == *command))
    }
}

/// What the lines were built from.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct View<'a> {
    pub query: &'a str,
    pub area: &'a AreaId,
    pub filter: Filter,
    pub collapsed: &'a BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct Key {
    query: String,
    area: AreaId,
    filter: Filter,
    collapsed: BTreeSet<String>,
    overrides: Overrides,
    platform: Platform,
}

/// The built lines, rebuilt only when the view or the keymap's overrides change.
#[derive(Clone, Debug, Default)]
pub(crate) struct RowCache {
    key: Option<Key>,
    built: Built,
}

impl RowCache {
    pub(crate) fn get(&mut self, keymap: &Keymap, options: &Options, text: &Say, view: &View) -> &Built {
        let fresh = self.key.as_ref().is_some_and(|k| {
            k.query == view.query
                && k.area == *view.area
                && k.filter == view.filter
                && k.collapsed == *view.collapsed
                && k.platform == keymap.platform()
                && k.overrides == *keymap.overrides()
        });
        if !fresh {
            self.built = build(keymap, options, text, view);
            self.key = Some(Key {
                query: view.query.to_string(),
                area: view.area.clone(),
                filter: view.filter,
                collapsed: view.collapsed.clone(),
                overrides: keymap.overrides().clone(),
                platform: keymap.platform(),
            });
        }
        &self.built
    }
}

fn top_menu(b: &CommandDef) -> Option<&str> {
    b.paths.first().and_then(|p| p.first()).map(String::as_str)
}

fn is_tool(b: &CommandDef) -> bool {
    matches!(b.kind, CommandKind::Tool { .. })
}

/// The catalog entries the table lists ([`Options::visible`]).
fn listed<'k>(keymap: &'k Keymap, options: &Options) -> impl Iterator<Item = &'k CommandDef> + Clone {
    let visible = options.visible;
    keymap.catalog().iter().filter(move |b| visible.is_none_or(|f| f(b)))
}

/// The Show entries, in catalog order, leaving out areas with no rows.
pub(crate) fn areas(keymap: &Keymap, options: &Options, text: &Say) -> Vec<AreaEntry> {
    let catalog = listed(keymap, options);
    let mut out = vec![AreaEntry { id: AreaId::All, label: text.words().all_commands.as_str().to_owned() }];
    let mut menus: Vec<&str> = Vec::new();
    for m in catalog.clone().filter_map(top_menu) {
        if !menus.contains(&m) {
            menus.push(m);
        }
    }
    for m in menus {
        let label = text.words().menu_menu.fill(&[("menu", &text.segment(m))]);
        out.push(AreaEntry { id: AreaId::Menu(m.to_string()), label });
    }
    if catalog.clone().any(is_tool) {
        out.push(AreaEntry { id: AreaId::Tools, label: text.words().tools.as_str().to_owned() });
    }
    let mut panels: Vec<&str> = keymap.scopes().panels().iter().map(String::as_str).collect();
    for b in catalog.clone().filter(|b| !b.scope.is_app()) {
        if !panels.contains(&b.scope.as_str()) {
            panels.push(b.scope.as_str());
        }
    }
    for p in panels.into_iter().filter(|p| catalog.clone().any(|b| b.scope.as_str() == *p)) {
        out.push(AreaEntry { id: AreaId::Panel(p.to_string()), label: panel_label(text, p) });
    }
    if catalog.clone().any(|b| top_menu(b).is_none() && !is_tool(b)) {
        out.push(AreaEntry { id: AreaId::Other, label: text.words().other.as_str().to_owned() });
    }
    for a in options.areas.iter().filter(|a| catalog.clone().any(a.rows)) {
        out.push(AreaEntry { id: AreaId::App(a.id), label: text.segment(a.label).into_owned() });
    }
    if !options.fixed.is_empty() {
        out.push(AreaEntry { id: AreaId::Fixed, label: text.words().fixed_keys.as_str().to_owned() });
    }
    out
}

fn panel_label(text: &Say, panel: &str) -> String {
    text.words().panel_panel.fill(&[("panel", &text.segment(panel))])
}

fn in_area(area: &AreaId, b: &CommandDef, options: &Options) -> bool {
    match area {
        AreaId::All => true,
        AreaId::Menu(m) => top_menu(b) == Some(m.as_str()),
        AreaId::Tools => is_tool(b),
        AreaId::Panel(p) => b.scope.as_str() == p,
        AreaId::Other => top_menu(b).is_none() && !is_tool(b),
        AreaId::App(id) => options.areas.iter().any(|a| a.id == *id && (a.rows)(b)),
        AreaId::Fixed => false,
    }
}

/// The first id segment, capitalized: `layer.lock` → `Layer`.
fn id_group(id: &str) -> String {
    let first = id.split('.').next().unwrap_or(id);
    let mut chars = first.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// A row's group header (`None`: no header) with its rank among groups, and how many path parts
/// the header stands for.
pub(crate) fn group(area: &AreaId, b: &CommandDef, submenus: bool, text: &Say) -> (u8, Option<String>, usize) {
    let path = b.paths.first().map(Vec::as_slice).unwrap_or_default();
    let menu = |depth| path.first().map(|m| (0, Some(text.segment(m).into_owned()), depth));
    match area {
        AreaId::All => menu(1).unwrap_or_else(|| {
            if let CommandKind::Tool { .. } = b.kind {
                (1, Some(text.words().tools.as_str().to_owned()), 0)
            } else if !b.scope.is_app() {
                (2, Some(panel_label(text, b.scope.as_str())), 0)
            } else {
                (3, Some(text.words().other.as_str().to_owned()), 0)
            }
        }),
        AreaId::Menu(_) if submenus && path.len() >= 3 => (0, path.get(1).map(|s| text.segment(s).into_owned()), 2),
        AreaId::Menu(_) => (0, None, 1),
        AreaId::Tools => match &b.kind {
            CommandKind::Tool { group: Some(g) } => (0, Some(text.segment(g).into_owned()), usize::MAX),
            _ => (0, None, usize::MAX),
        },
        AreaId::Other => (0, Some(id_group(&b.command.id)), usize::MAX),
        AreaId::Panel(_) | AreaId::App(_) | AreaId::Fixed => menu(1).unwrap_or((0, None, usize::MAX)),
    }
}

/// What a query asks for: lowercase text, and the trigger it spells, if any.
struct Query {
    text: String,
    trigger: Option<Trigger>,
}

impl Query {
    fn new(q: &str, platform: Platform) -> Query {
        let text = q.trim().to_lowercase();
        let trigger = Trigger::parse(q.trim()).ok().map(|t| t.effective(platform));
        Query { text, trigger }
    }

    fn hit(&self, s: &str) -> bool {
        s.to_lowercase().contains(&self.text)
    }

    /// Label, aliases, path parts and id, as written and translated; then the bindings: the same
    /// effective trigger when the query spells one, else their canonical or display text. A query
    /// that spells a chord (`cmd+s`, `⌘S`) does not also match longer chords containing its text.
    fn matches(&self, b: &CommandDef, bindings: &[Binding], text: &Say, platform: Platform) -> bool {
        if self.text.is_empty() {
            return true;
        }
        let named = self.hit(&b.label)
            || self.hit(&text.command(b))
            || b.aliases.iter().any(|a| self.hit(a))
            || b.paths.iter().flatten().any(|s| self.hit(s) || self.hit(&text.segment(s)))
            || self.hit(&b.command.id);
        named
            || match self.trigger {
                Some(t) => bindings.iter().any(|x| x.trigger.effective(platform) == t),
                None => bindings.iter().any(|x| {
                    self.hit(&x.trigger.to_string()) || self.hit(&x.trigger.display(platform)) || self.hit(&text.trigger(&x.trigger, platform))
                }),
            }
    }
}

fn build(keymap: &Keymap, options: &Options, text: &Say, view: &View) -> Built {
    let platform = keymap.platform();
    let catalog = keymap.catalog();
    let mut built = Built::default();
    let mut clashing: HashSet<String> = HashSet::new();
    for c in keymap.conflicts().into_iter().filter(|c| !keymap.twins_in(c)) {
        match c.kind {
            ConflictKind::Clash => {
                built.clashes += 1;
                clashing.insert(c.a.0.key());
                clashing.insert(c.b.0.key());
            }
            ConflictKind::Shadow => built.shadows += 1,
            _ => {}
        }
    }
    let query = Query::new(view.query, platform);
    if *view.area == AreaId::Fixed {
        build_fixed(keymap, options, text, view, &query, &mut built);
        return built;
    }

    let bindings: Vec<Vec<Binding>> = catalog.iter().map(|b| keymap.bindings(&b.command)).collect();
    // How many tools of a group hold each effective trigger.
    let mut tool_keys: HashMap<(Option<&str>, Trigger), usize> = HashMap::new();
    for (b, list) in catalog.iter().zip(&bindings) {
        if let CommandKind::Tool { group } = &b.kind {
            for x in list {
                *tool_keys.entry((group.as_deref().filter(|_| !keymap.shares_tool_keys()), x.trigger.effective(platform))).or_default() += 1;
            }
        }
    }
    let submenus = match view.area {
        AreaId::Menu(m) => listed(keymap, options).any(|b| top_menu(b) == Some(m.as_str()) && b.paths.first().is_some_and(|p| p.len() >= 3)),
        _ => false,
    };

    // Groups in first-seen order within their rank.
    let mut groups: Vec<(u8, Option<String>, Vec<Row>)> = Vec::new();
    for (index, (b, list)) in catalog.iter().zip(&bindings).enumerate() {
        let hidden = options.visible.is_some_and(|f| !f(b));
        if hidden || !in_area(view.area, b, options) || !query.matches(b, list, text, platform) {
            continue;
        }
        let key = b.command.key();
        let changed = keymap.overrides().get(&key).is_some();
        let conflict = clashing.contains(&key);
        let keep = match view.filter {
            Filter::All => true,
            Filter::WithShortcut => !list.is_empty(),
            Filter::Changed => changed,
            Filter::Conflicts => conflict,
        };
        if !keep {
            continue;
        }
        let (rank, header, depth) = group(view.area, b, submenus, text);
        let path = b.paths.first().map(Vec::as_slice).unwrap_or_default();
        let prefix = path.get(depth..path.len().saturating_sub(1)).unwrap_or_default().iter().map(|s| text.segment(s).into_owned()).collect();
        let defaults = &b.defaults;
        let chips = list
            .iter()
            .map(|x| {
                let mut t = text.trigger(&x.trigger, platform);
                if x.scope_or(&b.scope) != &b.scope {
                    let scope = text.segment(x.scope_or(&b.scope).as_str());
                    let s = text.words().chord_scope.fill(&[("chord", &t), ("scope", &scope)]);
                    t = around_chord(text.rtl(), &s, &t);
                }
                Chip { binding: x.clone(), text: t, changed: !defaults.contains(x) }
            })
            .collect();
        let cycles = match &b.kind {
            CommandKind::Tool { group } => list.iter().any(|x| {
                tool_keys.get(&(group.as_deref().filter(|_| !keymap.shares_tool_keys()), x.trigger.effective(platform))).is_some_and(|n| *n > 1)
            }),
            _ => false,
        };
        let unparsed = keymap.unparsed_texts(&b.command).into_iter().map(str::to_owned).collect();
        let row = Row { index, command: b.command.clone(), key, label: text.command(b), prefix, chips, unparsed, changed, cycles, conflict };
        match groups.iter_mut().find(|(r, h, _)| *r == rank && *h == header) {
            Some((_, _, rows)) => rows.push(row),
            None => groups.push((rank, header, vec![row])),
        }
    }
    groups.sort_by_key(|(rank, _, _)| *rank);
    for (_, header, mut rows) in groups {
        if options.sort_by_label {
            rows.sort_by_cached_key(|r| r.label.to_lowercase());
        }
        built.matches += rows.len();
        let collapsed = header.as_ref().is_some_and(|h| view.collapsed.contains(h));
        if let Some(name) = header {
            built.lines.push(Line::Header { name, collapsed });
        }
        if !collapsed {
            built.lines.extend(rows.into_iter().map(Line::Row));
        }
    }
    built
}

fn build_fixed(keymap: &Keymap, options: &Options, text: &Say, view: &View, query: &Query, built: &mut Built) {
    if matches!(view.filter, Filter::Changed | Filter::Conflicts) {
        return;
    }
    let platform = keymap.platform();
    let mut groups: Vec<(&str, Vec<FixedLine>)> = Vec::new();
    for f in &options.fixed {
        if !query.text.is_empty()
            && !(query.hit(&f.label)
                || query.hit(&f.keys)
                || query.hit(&f.group)
                || query.trigger.is_some_and(|t| f.chord.is_some_and(|c| c.effective(platform) == t)))
        {
            continue;
        }
        let now_used_by = f.chord.and_then(|c| {
            let c = c.effective(platform);
            keymap.catalog().iter().find(|b| keymap.bindings(&b.command).iter().any(|x| x.trigger.effective(platform) == c)).map(|b| text.command(b))
        });
        let line = FixedLine { label: f.label.clone(), keys: f.keys.clone(), now_used_by };
        match groups.iter_mut().find(|(g, _)| *g == f.group) {
            Some((_, lines)) => lines.push(line),
            None => groups.push((&f.group, vec![line])),
        }
    }
    for (name, lines) in groups {
        built.matches += lines.len();
        let collapsed = view.collapsed.contains(name);
        built.lines.push(Line::Header { name: name.to_string(), collapsed });
        if !collapsed {
            built.lines.extend(lines.into_iter().map(Line::Fixed));
        }
    }
}
