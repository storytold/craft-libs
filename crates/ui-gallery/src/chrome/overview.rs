//! The centre tabs: the Overview (the cards of one level), a component's tab (a card per story group) and the Variants view
//! that replaces the front one.

use super::card::{Card, CardEvents, CardStory, Look};
use super::panels::Panels;
use super::variants;
use crate::viewer::{Catalog, ComponentInfo, PanelId, TreeKey, by_level, entry_of};
use craft_behavior::{ColumnWidth, Selection};
use craft_theme::variant::Tone;
use craft_ui::contract::{Bind, BodyCx, Entry, Gap, Lazy, Pad, Props as _, Scroll, Show as _, ShowContainer as _, StoryLayout};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::{Grid, Row, Stack};
use craft_ui::notices::Notice;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::WindowKey;
use craft_ui::shell::AppShell;
use craft_ui::style::SurfaceKind;
use craft_ui::testing::StoryRegistration;
use egui::Ui;

/// The narrowest a card may be before the cards go to one column: a dock panel of the apps is about this wide, and a
/// story drawn in less is no longer what the apps show.
const CARD_MIN_WIDTH: f32 = 290.0;

/// A card's height before it ran, for the cards out of view (`Lazy`'s estimate): its header band, and a story row
/// as tall as a story of one control under its caption. Cards in view run, so the guess only places the cards past
/// them until those scroll in.
const HEADER_GUESS: f32 = 48.0;
const STORY_ROW_GUESS: f32 = 120.0;

const ONE: &[ColumnWidth] = &[ColumnWidth::fill(1.0)];
const TWO: &[ColumnWidth] = &[ColumnWidth::fill(1.0); 2];

/// One story of a card to show.
pub struct SpecStory {
    /// The index of the story in the catalog.
    pub index: usize,
    /// The story's registration.
    pub story: &'static StoryRegistration,
    /// The story's caption.
    pub caption: String,
}

impl SpecStory {
    /// The story at catalog index `index` of the component whose display name is `name`.
    pub fn of(catalog: &Catalog, index: usize, name: &str) -> Self {
        let story = catalog.entries[index].registration;
        SpecStory { index, story, caption: story.story.caption(name) }
    }
}

/// The card of one component to show, before it is laid out.
pub struct Spec {
    /// The id scope of the card.
    pub key: String,
    /// The registered name of the component.
    pub type_name: &'static str,
    /// The card's title.
    pub title: String,
    /// A note after the title.
    pub note: Option<String>,
    /// The stories the card shows, in order.
    pub stories: Vec<SpecStory>,
    /// How the stories are laid out together.
    pub layout: StoryLayout,
    /// The surface the stories sit on.
    pub surface: craft_ui::contract::Surface,
    /// Whether the header has Open.
    pub open: bool,
    /// Whether the header has Use.
    pub usable: bool,
    /// Whether the card has the selection ring.
    pub selected: bool,
    /// The values the card sets for its stories.
    pub look: Look,
    /// Whether the card gates its story's wheel takers (`Card::gated`).
    pub gated: bool,
}

impl Spec {
    /// The card's height before it ran, from its story rows: one per story, or per row of columns.
    fn estimate(&self) -> f32 {
        let per_row = match self.layout {
            StoryLayout::Columns(most) => usize::from(most.max(1)),
            _ => 1,
        };
        HEADER_GUESS + self.stories.len().div_ceil(per_row) as f32 * STORY_ROW_GUESS
    }

    /// The stories as the card takes them.
    pub fn card_stories(&self) -> Vec<CardStory<'_>> {
        self.stories.iter().map(|story| CardStory::new(story.index, story.story, &story.caption)).collect()
    }

    /// The card of this spec, with `stories` from [`Spec::card_stories`], `key` as its id scope and `room` as the
    /// height a filling story gets.
    pub fn card<'a>(&'a self, stories: &'a [CardStory<'a>], key: String, room: f32) -> Card<'a> {
        Card {
            key,
            title: &self.title,
            note: self.note.as_deref(),
            stories,
            layout: self.layout,
            surface: self.surface,
            room,
            open: self.open,
            usable: self.usable,
            selected: self.selected,
            look: self.look,
            gated: self.gated,
            inspect: None,
        }
    }
}

/// What the cards of a pass reported, by card.
pub(super) struct Reported {
    pub spec: usize,
    pub events: CardEvents,
}

/// The index in `catalog` of the story the component registered as `type_name` shows: the one picked for it, else its
/// first.
pub(super) fn target_story(p: &Panels<'_>, type_name: &'static str) -> Option<usize> {
    p.catalog.pick(type_name, p.story_of.get(type_name).copied())
}

/// The selected component's registered name.
pub(super) fn selected_type(p: &Panels<'_>) -> Option<&'static str> {
    p.selection.items.iter().rev().find_map(|key| match key {
        TreeKey::Component(type_name) => Some(*type_name),
        _ => None,
    })
}

/// The window's background (an embedded shell, which paints it and keeps its cards' dialogs in the tab) with the
/// panel's padding around `add`.
pub(super) fn on_background(ui: &mut Ui, key: &str, mut add: impl FnMut(&mut Ui, &mut BodyCx<'_>)) {
    AppShell::embedded(WindowKey::new(format!("gallery-centre-{key}")))
        .body(|ui, cx, _: &mut ()| {
            Stack::new().fill().surface(SurfaceKind::Background).pad(Pad::Regular).show(ui, |ui, _| add(ui, cx));
        })
        .show(ui, &mut ());
}

/// The cards in one or two columns, scrolling vertically; each row is as tall as its tallest card. A card out of view
/// does not run (`Lazy`); the card of the component `reveal` names glides to the top of the view. The selected card of
/// `tab` is inspected while the inspector is on. The cards report into `reported`.
pub(super) fn grid(
    ui: &mut Ui,
    scroll_id: impl std::hash::Hash + std::fmt::Debug,
    tab: Option<PanelId>,
    cards: &[Spec],
    p: &mut Panels<'_>,
    one_column: bool,
    reveal: Option<&'static str>,
) -> Vec<Reported> {
    let mut reported = Vec::new();
    let columns = if one_column || ui.available_width() < 2.0 * CARD_MIN_WIDTH { 1 } else { 2 };
    let generation = p.generation;
    // A story that fills its room gets the view's height, as on the old gallery's tiles.
    let room = ui.available_height();
    Stack::new().fill().scroll(Scroll::vertical()).gap(Gap::CardRow).id_salt(scroll_id).show(ui, |ui, _| {
        reported.clear();
        for (row, chunk) in cards.chunks(columns).enumerate() {
            Grid::new(if columns == 1 { ONE } else { TWO }).gap(Gap::CardColumn).fill_width().show(ui, |ui, _| {
                for (at, spec) in chunk.iter().enumerate() {
                    // Keyed by the card alone: a change of the column count regroups the rows, and the cards out of
                    // view keep their extents.
                    let lazy = Lazy::new((generation, spec.key.as_str()), spec.estimate()).reveal(reveal == Some(spec.type_name));
                    lazy.show(ui, |ui| {
                        let stories = spec.card_stories();
                        // Each card is its own id scope: a story's `id_salt` and auto ids must not meet another card's.
                        let mut card = spec.card(&stories, format!("{}-{generation}", spec.key), room);
                        card.inspect = p.inspect.of(tab, spec.selected);
                        let events = card.show(ui, p.cells);
                        reported.push(Reported { spec: row * columns + at, events });
                    });
                }
            });
        }
    });
    reported
}

/// Applies what the cards reported: a press selects the card's component and picks the card's first story unless the
/// picked one is on the card, Open opens its tab, Reset gives the card's stories their start values.
pub(super) fn apply(p: &mut Panels<'_>, cards: &[Spec], reported: Vec<Reported>) {
    for Reported { spec, events } in reported {
        let Some(spec) = cards.get(spec) else { continue };
        if events.reset {
            for story in &spec.stories {
                p.cells.entry(story.index).or_default().reset();
            }
        }
        if events.opened {
            *p.open_request = Some(spec.type_name);
        }
        if events.pressed || events.opened {
            *p.selection = Selection::single(TreeKey::Component(spec.type_name));
            // The picked story stays while it is one of the card's; else the card's first story is picked.
            let picked = target_story(p, spec.type_name);
            if let Some(first) = spec.stories.first()
                && !spec.stories.iter().any(|story| Some(story.index) == picked)
            {
                p.story_of.insert(spec.type_name, first.story.story.name);
            }
        }
    }
}

/// The selection, when it changed since the Overview's cards last followed it, and its component's level as the shown
/// one.
fn follow_selection(p: &mut Panels<'_>) -> Option<Option<&'static str>> {
    let selected = selected_type(p);
    if *p.followed == Some(selected) {
        return None;
    }
    if let Some(entry) = selected.and_then(entry_of) {
        *p.level = entry.level.title().to_owned();
    }
    Some(selected)
}

/// The Overview: the level select over the cards of the level's components.
pub(super) fn overview(ui: &mut Ui, p: &mut Panels<'_>) {
    if variants::show(ui, p, PanelId::Overview) {
        return;
    }
    let follow = follow_selection(p);
    let groups = by_level(p.catalog, p.filter);
    let Some(shown) = groups.iter().position(|(level, _)| level.title() == p.level.as_str()).or(if groups.is_empty() { None } else { Some(0) })
    else {
        on_background(ui, "overview", |ui, _| {
            Notice::new(Tone::Info, "No components match the filter.").show(ui);
        });
        return;
    };
    let level = groups[shown].0;
    let entries: Vec<Entry<craft_ui::contract::Level>> = groups.iter().map(|(level, _)| Entry::new(*level, level.title())).collect();
    let selected = selected_type(p);
    let cards: Vec<Spec> = groups[shown].1.iter().filter_map(|component| component_card(p, component, selected)).collect();
    let mut chosen = level;
    let mut reported = Vec::new();
    // The cards follow the selection in the pass they run: its card glides into view, and only then is it followed.
    let mut ran = false;
    on_background(ui, "overview", |ui, _| {
        ran = true;
        Row::new().show(ui, |ui, _| {
            Text::new("Level:").shade(Shade::Muted).show(ui);
            Select::new("Level", Bind::bound(&mut chosen), &entries).show(ui);
        });
        reported = grid(ui, level.title(), Some(PanelId::Overview), &cards, p, false, follow.flatten());
    });
    if ran && let Some(selected) = follow {
        *p.followed = Some(selected);
    }
    if chosen != level {
        *p.level = chosen.title().to_owned();
    }
    apply(p, &cards, reported);
}

/// The stories of group `group` of the component registered as `type_name`, under their captions.
fn group_stories(catalog: &Catalog, type_name: &'static str, group: usize, name: &str) -> Vec<SpecStory> {
    catalog.group_stories(type_name, group).into_iter().map(|at| SpecStory::of(catalog, at, name)).collect()
}

/// The group the component registered as `type_name` shows: the group of its picked story, else its first.
fn target_group(p: &Panels<'_>, type_name: &'static str) -> usize {
    target_story(p, type_name).and_then(|at| p.catalog.group_at(type_name, at)).unwrap_or(0)
}

/// The Overview's card of `component`: the cases of the group of its picked story; selected when it is the selected
/// component.
fn component_card(p: &Panels<'_>, component: &ComponentInfo, selected: Option<&'static str>) -> Option<Spec> {
    let mut spec = overview_card(p.catalog, component, target_group(p, component.type_name))?;
    spec.selected = selected == Some(component.type_name);
    Some(spec)
}

/// The Overview's card of `component` showing its story group `group`: the group's cases in catalog order, with the
/// group's name as a note when it is not the first group. Not selected.
pub fn overview_card(catalog: &Catalog, component: &ComponentInfo, group: usize) -> Option<Spec> {
    let entry = entry_of(component.type_name)?;
    let name = &component.groups.get(group)?.name;
    let stories = group_stories(catalog, component.type_name, group, &entry.name);
    if stories.is_empty() {
        return None;
    }
    Some(Spec {
        key: component.type_name.to_owned(),
        type_name: component.type_name,
        title: entry.name.clone(),
        note: (group > 0).then(|| name.clone()),
        stories,
        layout: entry.story_layout(name),
        surface: entry.surface,
        open: true,
        usable: false,
        selected: false,
        look: Look::default(),
        gated: true,
    })
}

/// A component's tab: each story group of the component as a card in one column, titled with the group's name.
pub(super) fn component_tab(ui: &mut Ui, p: &mut Panels<'_>, type_name: &'static str) {
    if variants::show(ui, p, PanelId::Component(type_name)) {
        return;
    }
    let (Some(entry), Some(component)) = (entry_of(type_name), p.catalog.component(type_name)) else {
        on_background(ui, "component", |ui, _| {
            Notice::new(Tone::Info, "The component has no stories.").show(ui);
        });
        return;
    };
    let target = target_group(p, type_name);
    let chosen = selected_type(p) == Some(type_name);
    let cards: Vec<Spec> = component
        .groups
        .iter()
        .enumerate()
        .map(|(at, group)| Spec {
            key: format!("{type_name}/{}", group.name),
            type_name,
            title: group.name.clone(),
            note: None,
            stories: group_stories(p.catalog, type_name, at, &entry.name),
            layout: entry.story_layout(&group.name),
            surface: entry.surface,
            open: false,
            usable: false,
            selected: chosen && target == at,
            look: Look::default(),
            gated: false,
        })
        .collect();
    let mut reported = Vec::new();
    on_background(ui, "component", |ui, _| reported = grid(ui, ("component", type_name), Some(PanelId::Component(type_name)), &cards, p, true, None));
    apply(p, &cards, reported);
}
