//! The bodies of the viewer's panels: the component tree, the target component's stories and facts, the inspector,
//! and the theme workspace's tokens, problems, detail and app sizes. The dock draws each panel's tab and frame.
//!
//! The tree, the stories and the properties are memoized views: each reads only its props, so it runs when a value it
//! reads changes and otherwise adds its last run again. The theme panels run every pass.

use super::describe;
use crate::viewer::{self, Browser, CardState, Catalog, ComponentInfo, PanelId, TreeKey, TreeRow, TreeRows};
use craft_behavior::IconId;
use craft_behavior::{Selection, keyed};
use craft_theme::Explained;
use craft_theme::report::ContrastReport;
use craft_theme_editor::{AppLayerEditor, Problems, ThemeSubject, ThemeWorkspace, TokenBrowser, TokenDetail};

use craft_ui::buttons::IconButton;
use craft_ui::contract::{
    Bind, CollectionEvent, MemoProps, MemoView, Outbox, Props as _, RowAlign, Scroll, SelectMode, Show as _, ShowCollection as _, ShowContainer as _,
    Tracked, bind_tracked,
};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::{List, ListRow, OpenSet, RowEvent, TrailingFace, TrailingPart};
use craft_ui::runtime::env::Label;
use craft_ui::sections::Section;
use craft_ui::style::TypeRole;
use craft_ui::testing::{StoryData, story_props};
use craft_ui::text::TextField;
use egui::Ui;
use std::collections::HashMap;
use std::sync::Arc;

/// The story each component's card shows, by the component's registered name and the story's name.
pub(super) type StoryOf = HashMap<&'static str, &'static str>;

/// The registered name of the component `selection` selects.
pub(super) fn selected_of(selection: &Selection<TreeKey>) -> Option<&'static str> {
    selection.items.iter().rev().find_map(|key| match key {
        TreeKey::Component(type_name) => Some(*type_name),
        _ => None,
    })
}

/// The index in `catalog` of the story the component registered as `type_name` shows: the one picked for it, else its
/// first.
pub(super) fn target_story(catalog: &Catalog, story_of: &StoryOf, type_name: &str) -> Option<usize> {
    catalog.pick(type_name, story_of.get(type_name).copied())
}

/// The id of the memoized view of `panel`'s body.
pub(super) fn view_id(panel: &PanelId) -> (&'static str, &PanelId) {
    ("gallery.panel", panel)
}

/// `add` filling the panel's body and scrolling vertically; the dock group insets the body. The body may run more than
/// once in a pass, so `add` is `FnMut`.
fn scrolled(ui: &mut Ui, mut add: impl FnMut(&mut Ui)) {
    Stack::new().fill().scroll(Scroll::vertical()).show(ui, |ui, _| add(ui));
}

/// The tree row part that opens a component's tab.
const OPEN: TrailingPart = TrailingPart::new(0);

/// A faint line where a panel has nothing to show yet.
fn hint(ui: &mut Ui, text: &'static str) {
    Text::new(text).shade(Shade::Faint).show(ui);
}

/// A caption over a value.
fn fact(ui: &mut Ui, name: &'static str, value: impl Into<Label>) {
    let value = value.into();
    Stack::new().fill_width().show(ui, |ui, _| {
        Text::new(name).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
        Text::new(value.clone()).show(ui);
    });
}

/// The declared facts of the component registered as `type_name`, the Inspect panel's last section: its decision
/// record, then where it is declared and what it is for.
pub(super) fn declared(ui: &mut Ui, catalog: &Catalog, type_name: Option<&'static str>) {
    let Some((type_name, declared)) = type_name.and_then(|type_name| Some((type_name, catalog.declared.get(type_name)?))) else {
        hint(ui, "Select a component");
        return;
    };
    Section::new("Declared").show(ui, |ui, _| {
        match declared.probe {
            None => hint(ui, "A pattern has no record of its own: its parts are the records."),
            Some(probe) => {
                fact(ui, "Shape", describe::shape(probe.shape));
                fact(ui, "Role", describe::role(probe.role));
                fact(ui, "Input", describe::input(probe.input));
                fact(ui, "Keys it keeps", describe::keys(&probe.keys));
                fact(ui, "Opens a popup", probe.overlay.to_string());
                fact(ui, "Shows its label", probe.shows_label.to_string());
                for exemption in probe.exempt {
                    fact(ui, "Exempt from", format!("{}: {}", exemption.case, exemption.reason));
                }
            }
        }
        fact(ui, "Type", type_name);
        fact(ui, "Module", declared.module);
        fact(ui, "Purpose", declared.purpose);
    });
}

/// What the component tree reads and edits.
#[derive(MemoProps)]
struct ComponentsProps<'a> {
    catalog: &'a Catalog,
    filter: &'a mut Tracked<String>,
    selection: &'a mut Tracked<Selection<TreeKey>>,
    open: &'a mut Tracked<OpenSet<TreeKey>>,
    /// The selected component the tree last opened the level of.
    seen: &'a mut Tracked<Option<Option<&'static str>>>,
    /// The components whose tab a row asked to open.
    opened: Outbox<'a, &'static str>,
}

/// The component tree: the filter over the levels and their components. Opening a row asks for the component's tab
/// through `opened`.
pub(super) fn components(ui: &mut Ui, browser: &mut Browser, opened: &mut Vec<&'static str>) {
    let Browser { catalog, filter, selection, open, tree_seen, .. } = browser;
    let props = ComponentsProps { catalog, filter, selection, open, seen: tree_seen, opened: Outbox::new(opened) };
    let _ = MemoView::new(view_id(&PanelId::Components), props, components_view).show(ui);
}

fn components_view(props: &mut ComponentsProps<'_>, ui: &mut Ui) {
    let ComponentsProps { catalog, filter, selection, open, seen, opened } = props;
    let before = selected_of(selection);
    viewer::open_level_of_selection(open, &catalog.components, before, seen);
    let shown = viewer::rows(&catalog.components, filter.as_str(), open);
    let groups = viewer::groups(&shown);
    let by_key: HashMap<&TreeKey, &TreeRow> = shown.iter().map(|row| (&row.key, row)).collect();
    let mut events = Vec::new();
    Stack::new().fill().show(ui, |ui, _| {
        TextField::search("Filter components", bind_tracked!(filter)).placeholder("Filter components").show(ui);
        let listed = List::<TreeKey>::new("Components")
            .select(SelectMode::Single)
            .type_ahead(true)
            .tree(true)
            .selection(bind_tracked!(selection))
            .empty("No component matches the filter")
            .show_items(ui, &TreeRows::new(&shown), &mut |key, cx| {
                let Some(row) = by_key.get(key) else { return ListRow::spacer() };
                let item = ListRow::new(row.label.clone()).depth(row.depth);
                if row.group {
                    return item.group(row.open);
                }
                // The Open icon shows on the selected row and the row under the pointer.
                if cx.row.selected || cx.row.hovered {
                    item.trailing_part(OPEN, TrailingFace::Icon(IconId::new("open").into()), format!("Open {}", row.label))
                } else {
                    item
                }
            });
        events = listed.events;
    });
    for event in events {
        match event {
            CollectionEvent::Open(change) => open.apply(&change.key, change.open, change.all, groups.iter()),
            CollectionEvent::Activate(TreeKey::Component(type_name))
            | CollectionEvent::Item(TreeKey::Component(type_name), RowEvent::Trailing { .. }) => {
                opened.send(type_name);
            }
            _ => {}
        }
    }
    // Selecting a component opens its level.
    let now = selected_of(selection);
    if now != before
        && let Some(component) = now.and_then(|type_name| catalog.component(type_name))
    {
        viewer::open_level(open, component.level);
    }
}

/// What the story list reads and edits.
#[derive(MemoProps)]
struct StoriesProps<'a> {
    catalog: &'a Catalog,
    selection: &'a Tracked<Selection<TreeKey>>,
    story_of: &'a mut Tracked<StoryOf>,
}

/// The story groups of the selected component; picking one shows it on the component's Overview card.
pub(super) fn stories(ui: &mut Ui, browser: &mut Browser) {
    let Browser { catalog, selection, story_of, .. } = browser;
    let props = StoriesProps { catalog, selection, story_of };
    let _ = MemoView::new(view_id(&PanelId::Stories), props, stories_view).show(ui);
}

fn stories_view(props: &mut StoriesProps<'_>, ui: &mut Ui) {
    let StoriesProps { catalog, selection, story_of } = props;
    let Some(component) = selected_of(selection).and_then(|type_name| catalog.component(type_name)) else {
        return scrolled(ui, |ui| {
            hint(ui, "Select a component");
        });
    };
    let (type_name, names) = (component.type_name, component.groups.iter().map(|group| group.name.clone()).collect::<Vec<_>>());
    let listed: Vec<usize> = (0..names.len()).collect();
    let shown = target_story(catalog, story_of, type_name).and_then(|at| catalog.group_at(type_name, at));
    let mut picked = Selection::default();
    picked.items.extend(shown);
    let before = picked.items.clone();
    Stack::new().fill().show(ui, |ui, _| {
        let source = keyed(&listed, |at| *at);
        List::<usize>::new("Stories")
            .select(SelectMode::Single)
            .selection(Bind::bound(&mut picked))
            .show_items(ui, &source, &mut |at, _| ListRow::new(names.get(*at).cloned().unwrap_or_default()));
    });
    if picked.items != before
        && let Some(&group) = picked.items.first()
        && let Some(&first) = catalog.group_stories(type_name, group).first()
    {
        let story = catalog.entries[first].registration.story.name;
        if story_of.get(type_name) != Some(&story) {
            story_of.insert(type_name, story);
        }
    }
}

/// What the Properties panel reads and edits: the target story and its state.
#[derive(MemoProps)]
struct PropertiesProps<'a> {
    catalog: &'a Catalog,
    /// The selected component and the catalog index of the story it shows.
    target: Option<(&'static str, usize)>,
    /// That story's state, keyed by its data's revision.
    state: Option<&'a mut CardState>,
}

/// The target story's name and its values' editors, with Reset args once a value changed.
pub(super) fn properties(ui: &mut Ui, browser: &mut Browser) {
    let target = browser.selected_type().zip(browser.target_story());
    let Browser { catalog, cells, .. } = browser;
    let state = target.map(|(_, index)| cells.state_of(index));
    let props = PropertiesProps { catalog, target, state };
    let _ = MemoView::new(view_id(&PanelId::Properties), props, properties_view).show(ui);
}

/// The display title of the story at catalog index `at` of `component`.
fn title(catalog: &Catalog, component: &ComponentInfo, at: usize) -> String {
    let place = catalog.stories_of(component.type_name).iter().position(|&story| story == at);
    place.and_then(|place| component.titles.get(place)).cloned().unwrap_or_default()
}

fn properties_view(props: &mut PropertiesProps<'_>, ui: &mut Ui) {
    let PropertiesProps { catalog, target, state } = props;
    let component = target.and_then(|(type_name, _)| catalog.component(type_name));
    let (Some(component), Some((_, index)), Some(state)) = (component, *target, state.as_deref_mut()) else {
        return scrolled(ui, |ui| {
            hint(ui, "Select a component");
        });
    };
    let title = title(catalog, component, index);
    // The values exist once the story has run; the first pass that finds them records the start.
    if !state.data.changed_since(&StoryData::default().snapshot()).is_empty() {
        state.note_start();
    }
    let changed = state.changed();
    let mut reset = false;
    scrolled(ui, |ui| {
        Row::new().fill_width().align(RowAlign::SpaceBetween).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                Text::new(component.name.clone()).role(TypeRole::Heading).show(ui);
                Text::new(title.clone()).shade(Shade::Muted).show(ui);
            });
            if changed {
                reset = IconButton::new(IconId::new("undo")).label("Reset args").show(ui).clicks > 0;
            }
        });
        story_props(ui, &mut state.data);
    });
    // An edit in the editors moves the revision the story's view keys on.
    state.data.settle();
    if reset {
        state.reset();
    }
}

/// The theme workspace's panels: what they read and edit in one pass. They run every pass.
#[non_exhaustive]
pub struct ThemePanels<'a> {
    /// What the theme panels share.
    pub themes: &'a mut ThemeWorkspace,
    /// The theme and appearance the theme panels explain.
    pub subject: ThemeSubject<'a>,
    /// The row the Problems view picked in this pass, which the token browser brings into view.
    pub revealed: Option<craft_theme_editor::Pick>,
}

impl ThemePanels<'_> {
    /// Shows the body of the theme panel `panel`; other panels show nothing.
    pub fn show(&mut self, ui: &mut Ui, panel: &PanelId) {
        match panel {
            PanelId::Problems => self.problems(ui),
            PanelId::Tokens => self.tokens(ui),
            PanelId::Detail => self.detail(ui),
            PanelId::AppSizes => self.overrides(ui),
            _ => {}
        }
    }

    /// A panel over the explanation and report of the subject.
    fn theme_panel(&mut self, ui: &mut Ui, mut body: impl FnMut(&mut Self, &mut Ui, &Arc<Explained>, &Arc<ContrastReport>)) {
        self.themes.refresh(&self.subject);
        let (Some(explained), Some(report)) = (self.themes.explained().cloned(), self.themes.report().cloned()) else { return };
        body(self, ui, &explained, &report);
    }

    fn problems(&mut self, ui: &mut Ui) {
        self.theme_panel(ui, |panels, ui, explained, report| {
            let picked = Problems::new(report, explained, &panels.subject).show(ui).picked;
            if let Some(pick) = picked {
                panels.themes.pick = Some(pick.clone());
                panels.revealed = Some(pick);
            }
        });
    }

    fn tokens(&mut self, ui: &mut Ui) {
        self.theme_panel(ui, |panels, ui, explained, _| {
            let mut browser = TokenBrowser::new(explained, Bind::bound(&mut panels.themes.pick));
            if let Some(pick) = &panels.revealed {
                browser = browser.reveal(pick);
            }
            browser.show(ui);
        });
    }

    fn detail(&mut self, ui: &mut Ui) {
        self.theme_panel(ui, |panels, ui, explained, _| {
            let mut detail = TokenDetail::new(explained);
            if let Some(pick) = &panels.themes.pick {
                detail = detail.pick(pick);
            }
            detail.show(ui);
        });
    }

    fn overrides(&mut self, ui: &mut Ui) {
        self.theme_panel(ui, |panels, ui, explained, _| {
            let pick = panels.themes.pick.clone();
            panels.themes.edit_sizes(|sizes| {
                let mut editor = AppLayerEditor::new(Bind::bound(sizes), explained);
                if let Some(pick) = &pick {
                    editor = editor.pick(pick);
                }
                editor.show(ui)
            });
        });
    }
}
