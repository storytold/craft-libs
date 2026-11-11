//! The bodies of the viewer's panels: the component tree, the target component's stories, facts and decision record,
//! and the theme workspace's tokens, problems, detail and app sizes. The dock draws each panel's tab and frame.

use super::describe;
use super::overview;
use crate::theme::{AppLayerEditor, Problems, ThemeSubject, ThemeWorkspace, TokenBrowser, TokenDetail};
use crate::viewer::{self, CardState, Catalog, Cells, ComponentInfo, PanelId, TreeKey, TreeRow, TreeRows};
use craft_behavior::IconId;
use craft_behavior::{Selection, keyed};
use craft_theme::Explained;
use craft_theme::report::ContrastReport;
use craft_ui::buttons::IconButton;
use craft_ui::contract::{Bind, CollectionEvent, Pad, Props as _, RowAlign, Scroll, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::{List, ListRow, OpenSet, RowEvent, TrailingFace, TrailingPart};
use craft_ui::runtime::env::Label;
use craft_ui::style::TypeRole;
use craft_ui::testing::{StoryData, story_props};
use craft_ui::text::TextField;
use egui::Ui;
use std::collections::HashMap;
use std::sync::Arc;

/// What the panels read and edit in one pass.
#[non_exhaustive]
pub struct Panels<'a> {
    /// What is listed.
    pub catalog: &'a Catalog,
    /// The text that narrows the tree.
    pub filter: &'a mut String,
    /// The selected row of the tree.
    pub selection: &'a mut Selection<TreeKey>,
    /// The open level groups.
    pub open: &'a mut OpenSet<TreeKey>,
    /// The story each component's card shows.
    pub story_of: &'a mut HashMap<&'static str, &'static str>,
    /// The shown stories' values.
    pub cells: &'a mut Cells,
    /// The component whose tab the tree asked to open in this pass.
    pub open_request: &'a mut Option<&'static str>,
    /// The component whose card the Overview brings into view when it is drawn.
    pub scroll_to: &'a mut Option<&'static str>,
    /// The selected component the Overview last followed to its level.
    pub followed: &'a mut Option<Option<&'static str>>,
    /// The selected component the tree last opened the level of.
    pub tree_seen: &'a mut Option<Option<&'static str>>,
    /// The reset count the cards' id scopes carry.
    pub generation: u32,
    /// What the theme panels share.
    pub themes: &'a mut ThemeWorkspace,
    /// The theme and appearance the theme panels explain.
    pub subject: ThemeSubject<'a>,
    /// The level the Overview shows, by its title.
    pub level: &'a mut String,
    /// The Variants view of the target story, while one is open.
    pub variants: &'a mut Option<crate::viewer::VariantAxis>,
    /// The centre tab the Variants view is open in.
    pub variants_tab: &'a mut Option<PanelId>,
    /// The root values; a Variants card that is used sets its value.
    pub env: &'a mut craft_ui::runtime::env::RootEnv,
    /// The row the Problems view picked in this pass, which the token browser brings into view.
    pub revealed: Option<crate::theme::Pick>,
}

/// `add` in the panel's padding, scrolling vertically. The body may run more than once in a pass, so `add` is `FnMut`.
fn scrolled(ui: &mut Ui, mut add: impl FnMut(&mut Ui)) {
    Stack::new().fill().pad(Pad::Regular).scroll(Scroll::vertical()).show(ui, |ui, _| add(ui));
}

/// `add` in the panel's padding, for a body that is a collection or scrolls itself. Like `scrolled`, `add` may run more
/// than once in a pass.
fn padded(ui: &mut Ui, mut add: impl FnMut(&mut Ui)) {
    Stack::new().fill().pad(Pad::Regular).show(ui, |ui, _| add(ui));
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

/// The values of the story at catalog index `index`: the one place the panels reach into the shown cells.
fn state_of(cells: &mut Cells, index: usize) -> &mut CardState {
    cells.entry(index).or_default()
}

impl Panels<'_> {
    /// Shows the body of `panel`.
    pub fn show(&mut self, ui: &mut Ui, panel: PanelId) {
        match panel {
            PanelId::Components => self.components(ui),
            PanelId::Stories => self.stories(ui),
            PanelId::Properties => self.properties(ui),
            PanelId::Inspect => self.record(ui),
            PanelId::Overview => overview::overview(ui, self),
            PanelId::Component(type_name) => overview::component_tab(ui, self, type_name),
            PanelId::Preview => {}
            PanelId::Problems => self.problems(ui),
            PanelId::Tokens => self.tokens(ui),
            PanelId::Detail => self.detail(ui),
            PanelId::AppSizes => self.overrides(ui),
        }
    }

    /// The selected component.
    fn selected(&self) -> Option<&ComponentInfo> {
        let type_name = self.selection.items.iter().rev().find_map(|key| match key {
            TreeKey::Component(type_name) => Some(*type_name),
            _ => None,
        })?;
        self.catalog.component(type_name)
    }

    fn components(&mut self, ui: &mut Ui) {
        let selected = self.selected().map(|component| component.type_name);
        viewer::open_level_of_selection(self.open, &self.catalog.components, selected, &mut *self.tree_seen);
        let shown = viewer::rows(&self.catalog.components, self.filter.as_str(), &*self.open);
        let groups = viewer::groups(&shown);
        let by_key: HashMap<&TreeKey, &TreeRow> = shown.iter().map(|row| (&row.key, row)).collect();
        let before = self.selected().map(|component| component.type_name);
        let mut opened = None;
        let (filter, selection, open) = (&mut *self.filter, &mut *self.selection, &mut *self.open);
        Stack::new().fill().pad(Pad::Regular).show(ui, |ui, _| {
            TextField::search("Filter components", Bind::bound(&mut *filter)).placeholder("Filter components").show(ui);
            let listed = List::<TreeKey>::new("Components")
                .select(SelectMode::Single)
                .tree(true)
                .selection(Bind::bound(&mut *selection))
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
            for event in listed.events {
                match event {
                    CollectionEvent::Open(change) => open.apply(&change.key, change.open, change.all, groups.iter()),
                    CollectionEvent::Activate(TreeKey::Component(type_name)) => opened = Some(type_name),
                    CollectionEvent::Item(TreeKey::Component(type_name), RowEvent::Trailing { .. }) => opened = Some(type_name),
                    _ => {}
                }
            }
        });
        if let Some(type_name) = opened {
            *self.open_request = Some(type_name);
        }
        // Selecting a component opens its level.
        let now = self.selected().map(|component| (component.type_name, component.level));
        if let Some((type_name, level)) = now
            && Some(type_name) != before
        {
            self.open.set(&TreeKey::Level(level), true);
        }
    }

    /// The story groups of the selected component; picking one shows it on the component's Overview card.
    fn stories(&mut self, ui: &mut Ui) {
        let catalog = self.catalog;
        let Some(component) = self.selected() else {
            return scrolled(ui, |ui| {
                hint(ui, "Select a component");
            });
        };
        let (type_name, names) = (component.type_name, component.groups.iter().map(|group| group.name.clone()).collect::<Vec<_>>());
        let listed: Vec<usize> = (0..names.len()).collect();
        let shown = self.target(type_name).and_then(|at| catalog.group_at(type_name, at));
        let mut selection = Selection::default();
        selection.items.extend(shown);
        let before = selection.items.clone();
        Stack::new().fill().pad(Pad::Regular).show(ui, |ui, _| {
            let source = keyed(&listed, |at| *at);
            List::<usize>::new("Stories")
                .select(SelectMode::Single)
                .selection(Bind::bound(&mut selection))
                .show_items(ui, &source, &mut |at, _| ListRow::new(names.get(*at).cloned().unwrap_or_default()));
        });
        if selection.items != before
            && let Some(&picked) = selection.items.first()
            && let Some(&first) = catalog.group_stories(type_name, picked).first()
        {
            self.story_of.insert(type_name, catalog.entries[first].registration.story.name);
        }
    }

    /// The index of the story the component registered as `type_name` shows: the one picked for it, else its first.
    fn target(&self, type_name: &'static str) -> Option<usize> {
        self.catalog.pick(type_name, self.story_of.get(type_name).copied())
    }

    /// The display title of the story at catalog index `at` of `component`.
    fn title(&self, component: &ComponentInfo, at: usize) -> String {
        let place = self.catalog.stories_of(component.type_name).iter().position(|&story| story == at);
        place.and_then(|place| component.titles.get(place)).cloned().unwrap_or_default()
    }

    fn properties(&mut self, ui: &mut Ui) {
        let component = self.selected().cloned();
        let target = component.as_ref().and_then(|component| self.target(component.type_name));
        let (Some(component), Some(index)) = (component, target) else {
            return scrolled(ui, |ui| {
                hint(ui, "Select a component");
            });
        };
        let title = self.title(&component, index);
        let state = state_of(self.cells, index);
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
        if reset {
            state.reset();
        }
    }

    /// The decision record of the selected component, then where it is declared and what it is for.
    fn record(&mut self, ui: &mut Ui) {
        let component = self.selected().map(|component| component.type_name);
        let declared = component.and_then(|type_name| self.catalog.declared.get(type_name));
        scrolled(ui, |ui| {
            let (Some(type_name), Some(declared)) = (component, declared) else {
                hint(ui, "Select a component");
                return;
            };
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

    /// A panel over the explanation and report of the subject.
    fn theme_panel(&mut self, ui: &mut Ui, mut body: impl FnMut(&mut Self, &mut Ui, &Arc<Explained>, &Arc<ContrastReport>)) {
        self.themes.refresh(&self.subject);
        let (Some(explained), Some(report)) = (self.themes.explained().cloned(), self.themes.report().cloned()) else { return };
        padded(ui, |ui| body(self, ui, &explained, &report));
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
