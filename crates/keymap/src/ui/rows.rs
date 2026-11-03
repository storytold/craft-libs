//! The table's model: areas (Show), filters, search, group headers, sorting and the cached list of
//! lines the table draws.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::chord::{Platform, Trigger};
use crate::command::CommandRef;
use crate::keymap::{Bindable, Binding, ConflictKind, Keymap, Kind, Overrides};
use crate::ui::Options;
use crate::ui::text::{Text, trigger_text, visual_around};

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

    pub(crate) fn label(self, text: &dyn Text) -> String {
        match self {
            Filter::All => text.ui("All"),
            Filter::WithShortcut => text.ui("With Shortcut"),
            Filter::Changed => text.ui("Changed"),
            Filter::Conflicts => text.ui("Conflicts"),
        }
        .into_owned()
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
    pub(crate) fn get(&mut self, keymap: &Keymap, options: &Options, text: &dyn Text, view: &View) -> &Built {
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

    /// The lines last built.
    pub(crate) fn built(&self) -> &Built {
        &self.built
    }
}

fn top_menu(b: &Bindable) -> Option<&str> {
    b.paths.first().and_then(|p| p.first()).map(String::as_str)
}

fn is_tool(b: &Bindable) -> bool {
    matches!(b.kind, Kind::Tool { .. })
}

/// The catalog entries the table lists ([`Options::visible`]).
fn listed<'k>(keymap: &'k Keymap, options: &Options) -> impl Iterator<Item = &'k Bindable> + Clone {
    let visible = options.visible;
    keymap.catalog().iter().filter(move |b| visible.is_none_or(|f| f(b)))
}

/// The Show entries, in catalog order, leaving out areas with no rows.
pub(crate) fn areas(keymap: &Keymap, options: &Options, text: &dyn Text) -> Vec<AreaEntry> {
    let catalog = listed(keymap, options);
    let mut out = vec![AreaEntry { id: AreaId::All, label: text.ui("All Commands").into_owned() }];
    let mut menus: Vec<&str> = Vec::new();
    for m in catalog.clone().filter_map(top_menu) {
        if !menus.contains(&m) {
            menus.push(m);
        }
    }
    for m in menus {
        let label = craft_i18n::fmt(&text.ui("{menu} Menu"), &[("menu", &text.segment(m))]);
        out.push(AreaEntry { id: AreaId::Menu(m.to_string()), label });
    }
    if catalog.clone().any(is_tool) {
        out.push(AreaEntry { id: AreaId::Tools, label: text.ui("Tools").into_owned() });
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
        out.push(AreaEntry { id: AreaId::Other, label: text.ui("Other").into_owned() });
    }
    for a in options.areas.iter().filter(|a| catalog.clone().any(a.rows)) {
        out.push(AreaEntry { id: AreaId::App(a.id), label: text.segment(a.label).into_owned() });
    }
    if !options.fixed.is_empty() {
        out.push(AreaEntry { id: AreaId::Fixed, label: text.ui("Fixed Keys").into_owned() });
    }
    out
}

fn panel_label(text: &dyn Text, panel: &str) -> String {
    craft_i18n::fmt(&text.ui("{panel} Panel"), &[("panel", &text.segment(panel))])
}

fn in_area(area: &AreaId, b: &Bindable, options: &Options) -> bool {
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
pub(crate) fn group(area: &AreaId, b: &Bindable, submenus: bool, text: &dyn Text) -> (u8, Option<String>, usize) {
    let path = b.paths.first().map(Vec::as_slice).unwrap_or_default();
    let menu = |depth| path.first().map(|m| (0, Some(text.segment(m).into_owned()), depth));
    match area {
        AreaId::All => menu(1).unwrap_or_else(|| {
            if let Kind::Tool { .. } = b.kind {
                (1, Some(text.ui("Tools").into_owned()), 0)
            } else if !b.scope.is_app() {
                (2, Some(panel_label(text, b.scope.as_str())), 0)
            } else {
                (3, Some(text.ui("Other").into_owned()), 0)
            }
        }),
        AreaId::Menu(_) if submenus && path.len() >= 3 => (0, path.get(1).map(|s| text.segment(s).into_owned()), 2),
        AreaId::Menu(_) => (0, None, 1),
        AreaId::Tools => match &b.kind {
            Kind::Tool { group: Some(g) } => (0, Some(text.segment(g).into_owned()), usize::MAX),
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
    fn matches(&self, b: &Bindable, bindings: &[Binding], text: &dyn Text, platform: Platform) -> bool {
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
                    self.hit(&x.trigger.to_string()) || self.hit(&x.trigger.display(platform)) || self.hit(&trigger_text(text, &x.trigger, platform))
                }),
            }
    }
}

fn build(keymap: &Keymap, options: &Options, text: &dyn Text, view: &View) -> Built {
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
        if let Kind::Tool { group } = &b.kind {
            for x in list {
                *tool_keys.entry((group.as_deref(), x.trigger.effective(platform))).or_default() += 1;
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
                let mut t = trigger_text(text, &x.trigger, platform);
                if x.scope_or(&b.scope) != &b.scope {
                    let scope = text.segment(x.scope_or(&b.scope).as_str());
                    let s = craft_i18n::fmt(&text.ui("{chord} ({scope})"), &[("chord", &t), ("scope", &scope)]);
                    t = visual_around(text, &s, &t);
                }
                Chip { binding: x.clone(), text: t, changed: !defaults.contains(x) }
            })
            .collect();
        let cycles = match &b.kind {
            Kind::Tool { group } => list.iter().any(|x| tool_keys.get(&(group.as_deref(), x.trigger.effective(platform))).is_some_and(|n| *n > 1)),
            Kind::Command => false,
        };
        let row = Row { index, command: b.command.clone(), key, label: text.command(b).into_owned(), prefix, chips, changed, cycles, conflict };
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

fn build_fixed(keymap: &Keymap, options: &Options, text: &dyn Text, view: &View, query: &Query, built: &mut Built) {
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
            keymap
                .catalog()
                .iter()
                .find(|b| keymap.bindings(&b.command).iter().any(|x| x.trigger.effective(platform) == c))
                .map(|b| text.command(b).into_owned())
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

#[cfg(test)]
pub(crate) mod tests {
    use std::borrow::Cow;

    use super::*;
    use crate::keymap::Bindable;
    use crate::scope::{Scope, Scopes};
    use crate::ui::text::English;
    use crate::ui::{Area, FixedRow};

    fn key(text: &str) -> Binding {
        Binding::parse(text).unwrap_or_else(|_| Binding::new(Trigger::single(crate::Chord::from(crate::Key::F35))))
    }

    /// The fixture of the dialog tests: Save, Save As, Export, Arrange › Bring to Front, two
    /// tools sharing M, Timeline Loop, Bold in the Text context.
    pub(crate) fn fixture(platform: Platform) -> Keymap {
        let tool = Kind::Tool { group: Some("Marquee".into()) };
        let path = |p: &[&str]| p.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let timeline = Scope::new("Timeline").unwrap_or_else(Scope::app);
        let catalog = vec![
            Bindable::new("file.save", "Save", Scope::app()).with_path(path(&["File", "Save"])).with_default(key("Cmd+S")),
            Bindable::new("file.saveAs", "Save As…", Scope::app()).with_path(path(&["File", "Save As…"])).with_default(key("Cmd+Shift+S")),
            Bindable::new("file.export", "Export…", Scope::app()).with_path(path(&["File", "Export…"])).with_default(key("Cmd+E")),
            Bindable::new("object.front", "Bring to Front", Scope::app()).with_path(path(&["Object", "Arrange", "Bring to Front"])),
            Bindable::new("tool.marquee", "Marquee Tool", Scope::app()).with_kind(tool.clone()).with_default(key("M")),
            Bindable::new("tool.ellipse", "Elliptical Marquee Tool", Scope::app()).with_kind(tool).with_default(key("M")),
            Bindable::new("timeline.loop", "Loop", timeline).with_default(key("L")),
            Bindable::new("type.bold", "Bold", Scope::app()).with_default(key("Text: Cmd+B")),
        ];
        Keymap::new(catalog, Scopes::new().with_panel("Timeline").with_context("Text"), platform)
    }

    fn view<'a>(query: &'a str, area: &'a AreaId, filter: Filter, collapsed: &'a BTreeSet<String>) -> View<'a> {
        View { query, area, filter, collapsed }
    }

    fn labels(built: &Built) -> Vec<String> {
        built
            .lines
            .iter()
            .map(|l| match l {
                Line::Header { name, .. } => format!("# {name}"),
                Line::Row(r) => {
                    let chips: Vec<&str> = r.chips.iter().map(|c| c.text.as_str()).collect();
                    let cycles = if r.cycles { " (cycles)" } else { "" };
                    format!("{}{}: {}{cycles}", r.prefix.iter().map(|p| format!("{p} › ")).collect::<String>(), r.label, chips.join(", "))
                }
                Line::Fixed(f) => format!("{} = {}{}", f.label, f.keys, f.now_used_by.as_ref().map(|l| format!(" (now {l})")).unwrap_or_default()),
            })
            .collect()
    }

    fn lines(km: &Keymap, options: &Options, text: &dyn Text, query: &str, area: AreaId, filter: Filter) -> Vec<String> {
        let none = BTreeSet::new();
        labels(&build(km, options, text, &view(query, &area, filter, &none)))
    }

    #[test]
    fn areas_follow_paths_kind_and_scope() {
        let km = fixture(Platform::Mac);
        let options = Options { areas: vec![Area { id: "type", label: "Type", rows: |b| b.command.id.starts_with("type.") }], ..Options::default() };
        let names: Vec<String> = areas(&km, &options, &English).into_iter().map(|a| a.label).collect();
        assert_eq!(names, ["All Commands", "File Menu", "Object Menu", "Tools", "Timeline Panel", "Other", "Type"]);

        let all = lines(&km, &options, &English, "", AreaId::All, Filter::All);
        assert_eq!(
            all,
            [
                "# File",
                "Save: ⌘S",
                "Save As…: ⇧⌘S",
                "Export…: ⌘E",
                "# Object",
                "Arrange › Bring to Front: ",
                "# Tools",
                "Marquee Tool: M (cycles)",
                "Elliptical Marquee Tool: M (cycles)",
                "# Timeline Panel",
                "Loop: L",
                "# Other",
                "Bold: ⌘B (Text)",
            ]
        );
        assert_eq!(lines(&km, &options, &English, "", AreaId::Menu("Object".into()), Filter::All), ["# Arrange", "Bring to Front: "]);
        assert_eq!(lines(&km, &options, &English, "", AreaId::Panel("Timeline".into()), Filter::All), ["Loop: L"]);
        assert_eq!(lines(&km, &options, &English, "", AreaId::Other, Filter::All), ["# Timeline", "Loop: L", "# Type", "Bold: ⌘B (Text)"]);
    }

    #[test]
    fn sort_by_label_and_collapsed_groups() {
        let km = fixture(Platform::Mac);
        let options = Options { sort_by_label: true, ..Options::default() };
        let area = AreaId::Menu("File".into());
        let none = BTreeSet::new();
        assert_eq!(labels(&build(&km, &options, &English, &view("", &area, Filter::All, &none))), ["Export…: ⌘E", "Save: ⌘S", "Save As…: ⇧⌘S"]);
        let collapsed: BTreeSet<String> = ["File".to_string()].into();
        let built = build(&km, &options, &English, &view("", &AreaId::All, Filter::All, &collapsed));
        assert_eq!(built.lines.first(), Some(&Line::Header { name: "File".into(), collapsed: true }));
        assert!(matches!(built.lines.get(1), Some(Line::Header { .. })));
    }

    struct Spanish;
    impl Text for Spanish {
        fn ui<'a>(&'a self, source: &'a str) -> Cow<'a, str> {
            source.into()
        }
        fn plural(&self, n: u64, one: &str, other: &str) -> String {
            English.plural(n, one, other)
        }
        fn command<'a>(&'a self, b: &'a Bindable) -> Cow<'a, str> {
            if b.command.id == "file.export" { "Exportar…".into() } else { b.label.as_str().into() }
        }
    }

    #[test]
    fn search_by_name_translation_or_chord() {
        let km = fixture(Platform::Mac);
        let o = Options::default();
        let find = |q: &str| -> Vec<String> {
            let none = BTreeSet::new();
            build(&km, &o, &Spanish, &view(q, &AreaId::All, Filter::All, &none))
                .lines
                .into_iter()
                .filter_map(|l| if let Line::Row(r) = l { Some(r.key) } else { None })
                .collect()
        };
        assert_eq!(find("save"), ["file.save", "file.saveAs"]);
        assert_eq!(find("exportar"), ["file.export"]);
        assert_eq!(find("cmd+s"), ["file.save"]);
        assert_eq!(find("⌘S"), ["file.save"]);
        assert_eq!(find("⇧⌘S"), ["file.saveAs"]);
        assert_eq!(find("Text: Cmd+B"), Vec::<String>::new(), "a binding text with a scope is not a trigger");
        assert_eq!(find("cmd+b"), ["type.bold"], "a trigger matches in any scope");
        assert_eq!(find("arrange"), ["object.front"]);
        assert!(find("zzz").is_empty());
    }

    #[test]
    fn filters_and_conflict_counts() -> Result<(), crate::AssignError> {
        let mut km = fixture(Platform::Windows);
        let o = Options::default();
        km.assign(&"file.export".into(), key("Cmd+S"), crate::Policy::Keep)?;
        assert_eq!(lines(&km, &o, &English, "", AreaId::All, Filter::Changed), ["# File", "Export…: Ctrl+E, Ctrl+S"]);
        assert_eq!(lines(&km, &o, &English, "", AreaId::All, Filter::Conflicts), ["# File", "Save: Ctrl+S", "Export…: Ctrl+E, Ctrl+S"]);
        assert_eq!(lines(&km, &o, &English, "", AreaId::Menu("Object".into()), Filter::WithShortcut), Vec::<String>::new());
        let none = BTreeSet::new();
        let built = build(&km, &o, &English, &view("", &AreaId::All, Filter::All, &none));
        assert_eq!((built.clashes, built.matches), (1, 8));
        Ok(())
    }

    #[test]
    fn import_paths_make_no_areas_or_groups() {
        let tool = |id: &str, label: &str| {
            Bindable::new(id, label, Scope::app())
                .with_kind(Kind::Tool { group: None })
                .with_import_path(vec!["Tools".into(), label.into()])
                .with_default(key("P"))
        };
        let km = Keymap::new(vec![tool("tool.pen", "Pen Tool")], Scopes::new(), Platform::Mac);
        let o = Options::default();
        let names: Vec<String> = areas(&km, &o, &English).into_iter().map(|a| a.label).collect();
        assert_eq!(names, ["All Commands", "Tools"]);
        assert_eq!(lines(&km, &o, &English, "", AreaId::All, Filter::All), ["# Tools", "Pen Tool: P"]);
    }

    #[test]
    fn hidden_entries_are_not_listed_but_still_conflict() -> Result<(), crate::AssignError> {
        let mut km = fixture(Platform::Mac);
        let o = Options { visible: Some(|b| b.command.id != "file.export" && !b.command.id.starts_with("timeline.")), ..Options::default() };
        let names: Vec<String> = areas(&km, &o, &English).into_iter().map(|a| a.label).collect();
        assert_eq!(names, ["All Commands", "File Menu", "Object Menu", "Tools", "Other"]);
        assert_eq!(lines(&km, &o, &English, "", AreaId::Menu("File".into()), Filter::All), ["Save: ⌘S", "Save As…: ⇧⌘S"]);
        assert!(lines(&km, &o, &English, "export", AreaId::All, Filter::All).is_empty());

        km.assign(&"file.save".into(), key("Cmd+E"), crate::Policy::Keep)?;
        let none = BTreeSet::new();
        assert_eq!(build(&km, &o, &English, &view("", &AreaId::All, Filter::All, &none)).clashes, 1);
        assert_eq!(lines(&km, &o, &English, "", AreaId::All, Filter::Conflicts), ["# File", "Save: ⌘S, ⌘E"]);
        Ok(())
    }

    #[test]
    fn commands_sharing_a_default_key_are_no_conflict() -> Result<(), crate::AssignError> {
        let menu = Bindable::new("app.characterPanel", "Character", Scope::app()).with_path(vec!["Window".into(), "Character".into()]);
        let catalog = vec![
            menu.with_default(key("Cmd+T")),
            Bindable::new("panel.character", "Character Panel", Scope::app()).with_default(key("Cmd+T")),
            Bindable::new("file.save", "Save", Scope::app()).with_default(key("Cmd+S")),
        ];
        let mut km = Keymap::new(catalog, Scopes::new(), Platform::Mac);
        let o = Options::default();
        let none = BTreeSet::new();
        let built = build(&km, &o, &English, &view("", &AreaId::All, Filter::All, &none));
        assert_eq!((built.clashes, built.shadows), (0, 0));
        assert!(lines(&km, &o, &English, "", AreaId::All, Filter::Conflicts).is_empty());

        km.assign(&"file.save".into(), key("Cmd+T"), crate::Policy::Keep)?;
        let built = build(&km, &o, &English, &view("", &AreaId::All, Filter::All, &none));
        assert_eq!(built.clashes, 2, "Save clashes with both twins");
        let flagged: Vec<String> =
            built.lines.iter().filter_map(|l| if let Line::Row(r) = l { r.conflict.then(|| r.key.clone()) } else { None }).collect();
        assert_eq!(flagged, ["app.characterPanel", "panel.character", "file.save"]);
        Ok(())
    }

    #[test]
    fn fixed_rows_strike_through_when_a_command_takes_their_chord() {
        let km = fixture(Platform::Mac);
        let fixed = |label: &str, keys: &str, chord: Option<&str>| FixedRow {
            group: "Rating".into(),
            label: label.into(),
            keys: keys.into(),
            chord: chord.and_then(|c| Trigger::parse(c).ok()),
        };
        let o = Options { fixed: vec![fixed("Rate 1", "1", Some("1")), fixed("Loop", "L", Some("L"))], ..Options::default() };
        assert_eq!(lines(&km, &o, &English, "", AreaId::Fixed, Filter::All), ["# Rating", "Rate 1 = 1", "Loop = L (now Loop)"]);
    }

    #[test]
    fn cache_rebuilds_on_overrides_and_view_changes() -> Result<(), crate::AssignError> {
        let mut km = fixture(Platform::Mac);
        let o = Options::default();
        let mut cache = RowCache::default();
        let none = BTreeSet::new();
        let area = AreaId::Menu("File".into());
        let before = cache.get(&km, &o, &English, &view("", &area, Filter::All, &none)).clone();
        km.assign(&"file.save".into(), key("F2"), crate::Policy::Steal)?;
        let after = cache.get(&km, &o, &English, &view("", &area, Filter::All, &none)).clone();
        assert_ne!(before, after);
        assert_eq!(cache.get(&km, &o, &English, &view("save as", &area, Filter::All, &none)).matches, 1);
        Ok(())
    }
}
