//! What the viewer lists: every registered story, grouped by the component it shows.

use craft_ui::contract::{self, Level, Probe, Registration};
use craft_ui::testing::{StoryRegistration, stories};
use std::collections::HashMap;
use std::sync::OnceLock;

/// One registered story with the facts the viewer lists.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct Entry {
    /// The registration.
    pub registration: &'static StoryRegistration,
    /// The registered name of the type the story shows.
    pub type_name: &'static str,
}

/// The facts of one component the tree lists and the filter reads.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ComponentInfo {
    /// The registered type name.
    pub type_name: &'static str,
    /// The level the tree lists the component under.
    pub level: Level,
    /// The component's plain name.
    pub name: String,
    /// The names of the component's stories.
    pub stories: Vec<&'static str>,
    /// The display titles of the stories, in the order of `stories`.
    pub titles: Vec<String>,
    /// The story groups, in the order of their first story.
    pub groups: Vec<StoryGroup>,
    /// The name, the titles and the group names in lower case, for the filter.
    folded: Vec<String>,
    /// The registry's level of the type and its display name in lower case; `None` when the registry does not list it.
    listed: Option<(Level, String)>,
}

/// A named group of one component's stories, shown together as the cases of one old gallery story.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct StoryGroup {
    /// The group's name (`Story::group_name`).
    pub name: String,
    /// The group's stories, as places in the component's `stories`, in declaration order.
    pub members: Vec<usize>,
}

impl ComponentInfo {
    /// A component with its stories, each a name and a display title, and each its own group under its title.
    pub fn new(type_name: &'static str, level: Level, name: impl Into<String>, stories: &[(&'static str, &str)]) -> Self {
        let name = name.into();
        let listed = linked_catalog().entry(type_name).map(|entry| (entry.level, entry.name.to_lowercase()));
        let mut info = ComponentInfo {
            type_name,
            level,
            folded: vec![name.to_lowercase()],
            name,
            stories: Vec::new(),
            titles: Vec::new(),
            groups: Vec::new(),
            listed,
        };
        for (story, title) in stories {
            info.push(story, (*title).to_owned(), (*title).to_owned());
        }
        info
    }

    /// Adds `story` under `title` to the group named `group`, which is new unless an earlier story named it.
    fn push(&mut self, story: &'static str, title: String, group: String) {
        let place = self.stories.len();
        self.stories.push(story);
        self.folded.push(title.to_lowercase());
        self.titles.push(title);
        match self.groups.iter_mut().find(|known| known.name == group) {
            Some(known) => known.members.push(place),
            None => {
                self.folded.push(group.to_lowercase());
                self.groups.push(StoryGroup { name: group, members: vec![place] });
            }
        }
    }

    /// The group that holds the story at `place` in `stories`.
    pub fn group_of(&self, place: usize) -> Option<usize> {
        self.groups.iter().position(|group| group.members.contains(&place))
    }

    /// Whether the name, one of the story titles or one of the group names contains the filter, ignoring case;
    /// `folded` is the filter as [`fold_filter`] gives it. An empty filter matches.
    pub fn matches(&self, folded: &str) -> bool {
        folded.is_empty() || self.folded.iter().any(|text| text.contains(folded))
    }

    /// The registry's level of the type and its display name in lower case, the Overview's order within the level.
    pub(crate) fn listed(&self) -> Option<(Level, &str)> {
        self.listed.as_ref().map(|(level, name)| (*level, name.as_str()))
    }
}

/// The filter as [`ComponentInfo::matches`] compares it: trimmed and in lower case.
pub fn fold_filter(filter: &str) -> String {
    filter.trim().to_lowercase()
}

/// The registry's catalog of the types linked into the running binary: each one's level and plain name.
pub(crate) fn linked_catalog() -> &'static contract::Catalog {
    static CATALOG: OnceLock<contract::Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| contract::catalog(&contract::registrations()))
}

/// The plain name of the component registered as `type_name`; the registered name when the registry lists no such type.
pub fn display_name(type_name: &str) -> String {
    linked_catalog().entry(type_name).map_or_else(|| type_name.to_owned(), |entry| entry.name.clone())
}

/// Every story linked into the viewer, ordered by type then story name.
pub fn entries() -> Vec<Entry> {
    stories().into_iter().map(|registration| Entry { registration, type_name: (registration.type_name)() }).collect()
}

/// What a registration says about a type: where it is declared and what it is for.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct Declared {
    /// The module path of the declaration.
    pub module: &'static str,
    /// The first sentence of the type's doc comment.
    pub purpose: &'static str,
    /// The component probe; `None` for a pattern.
    pub probe: Option<&'static Probe>,
}

/// The registration of each registered component and pattern type, by the type's registered name.
pub fn declared() -> HashMap<&'static str, Declared> {
    let mut out = HashMap::new();
    for registration in contract::registrations() {
        match registration {
            Registration::Component(probe) => {
                out.insert(probe.type_name(), Declared { module: probe.module, purpose: probe.purpose, probe: Some(probe) });
            }
            Registration::Pattern(probe) => {
                out.insert(probe.type_name(), Declared { module: probe.module, purpose: probe.purpose, probe: None });
            }
            _ => {}
        }
    }
    out
}

/// The components of `entries` that the registry lists, in the order of their first story.
pub fn components(entries: &[Entry]) -> Vec<ComponentInfo> {
    let mut out: Vec<ComponentInfo> = Vec::new();
    for entry in entries {
        let Some(listed) = linked_catalog().entry(entry.type_name) else { continue };
        let story = &entry.registration.story;
        let (title, group) = (story.caption(&listed.name), story.group_name(&listed.name));
        let at = match out.iter().position(|component| component.type_name == entry.type_name) {
            Some(at) => at,
            None => {
                out.push(ComponentInfo::new(entry.type_name, listed.level, listed.name.clone(), &[]));
                out.len() - 1
            }
        };
        out[at].push(story.name, title, group);
    }
    out
}

/// Every registered story, the components they show and what the registrations say about each type.
#[non_exhaustive]
pub struct Catalog {
    /// The stories, ordered by type then story name.
    pub entries: Vec<Entry>,
    /// The components of the stories.
    pub components: Vec<ComponentInfo>,
    /// The registration of each type.
    pub declared: HashMap<&'static str, Declared>,
    /// Each component's place in `components` and the catalog indices of its stories, in the order of its `stories`.
    by_type: HashMap<&'static str, (usize, Vec<usize>)>,
}

impl Catalog {
    /// The catalog of the stories linked into the running binary.
    pub fn linked() -> Self {
        let entries = entries();
        let declared = declared();
        let components = components(&entries);
        Catalog::new(entries, components, declared)
    }

    /// A catalog of `components` with no stories to show.
    pub fn listing(components: Vec<ComponentInfo>) -> Self {
        Catalog::new(Vec::new(), components, HashMap::new())
    }

    fn new(entries: Vec<Entry>, components: Vec<ComponentInfo>, declared: HashMap<&'static str, Declared>) -> Self {
        let mut by_type: HashMap<&'static str, (usize, Vec<usize>)> =
            components.iter().enumerate().map(|(at, component)| (component.type_name, (at, Vec::new()))).collect();
        for (at, entry) in entries.iter().enumerate() {
            if let Some((_, stories)) = by_type.get_mut(entry.type_name) {
                stories.push(at);
            }
        }
        Catalog { entries, components, declared, by_type }
    }

    /// The component registered as `type_name`.
    pub fn component(&self, type_name: &str) -> Option<&ComponentInfo> {
        self.by_type.get(type_name).map(|(at, _)| &self.components[*at])
    }

    /// The catalog indices of the stories of the component registered as `type_name`, in the order of its `stories`.
    pub fn stories_of(&self, type_name: &str) -> &[usize] {
        self.by_type.get(type_name).map_or(&[], |(_, stories)| stories.as_slice())
    }

    /// The catalog index of the story of the component registered as `type_name` that a view shows: the one named
    /// `picked`, else the component's first.
    pub fn pick(&self, type_name: &str, picked: Option<&str>) -> Option<usize> {
        let stories = self.stories_of(type_name);
        picked
            .and_then(|name| stories.iter().copied().find(|&at| self.entries[at].registration.story.name == name))
            .or_else(|| stories.first().copied())
    }

    /// The catalog indices of the stories of group `group` of the component registered as `type_name`.
    pub fn group_stories(&self, type_name: &str, group: usize) -> Vec<usize> {
        let (Some(component), all) = (self.component(type_name), self.stories_of(type_name)) else { return Vec::new() };
        component.groups.get(group).map_or_else(Vec::new, |group| group.members.iter().filter_map(|&place| all.get(place).copied()).collect())
    }

    /// The group of the component registered as `type_name` that holds the story at catalog index `story`.
    pub fn group_at(&self, type_name: &str, story: usize) -> Option<usize> {
        let place = self.stories_of(type_name).iter().position(|&at| at == story)?;
        self.component(type_name)?.group_of(place)
    }
}
