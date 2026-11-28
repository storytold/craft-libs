//! The centre tabs: the Overview (the cards of one level), a component's tab (a card per story group) and the Variants view
//! that replaces the front one. Each tab's body is a memoized view around its cards, which are memoized views of their
//! own.

use super::card::{Card, CardEvents, CardStory, Look};
use super::inspect::Inspected;
use super::panels::{StoryOf, selected_of, target_story, view_id};
use super::variants::{self, VariantsEvent, VariantsIn};
use crate::viewer::{Catalog, Cells, ComponentInfo, PanelId, TreeKey, by_level, entry_of};
use craft_behavior::{ColumnWidth, Selection};
use craft_theme::variant::Tone;
use craft_theme::{Tokens, TypeRole};
use craft_ui::contract::{
    Bind, BodyCx, Entry, Gap, Lazy, MemoProps, MemoView, Outbox, Pad, Props as _, Scroll, Show as _, ShowContainer as _, Sizing, StoryLayout, Tracked,
};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::{Grid, Row, Stack};
use craft_ui::notices::Notice;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, WindowKey};
use craft_ui::shell::AppShell;
use craft_ui::style::SurfaceKind;
use craft_ui::testing::{StoryRegistration, least_stage_width};
use egui::Ui;

/// A card's story row before it ran, in standard control heights (120 at a 24 control).
const STORY_ROW_IN_CONTROLS: f32 = 5.0;

/// The narrowest a card may be before the cards go to one column: a story at its least width inside the card's padding.
fn card_min_width(tokens: &Tokens) -> f32 {
    least_stage_width(tokens) + 2.0 * tokens.space.pad.regular
}

const ONE: &[ColumnWidth] = &[ColumnWidth::fill(1.0)];
const TWO: &[ColumnWidth] = &[ColumnWidth::fill(1.0); 2];

/// One story of a card to show.
#[non_exhaustive]
pub struct SpecStory {
    /// The index of the story in the catalog.
    pub index: usize,
    /// The story's registration.
    pub story: StoryRegistration,
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
#[non_exhaustive]
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
    /// The card's height before it ran, for the rows out of view (`Lazy`'s estimate is the tallest of the row's): its
    /// header band (a title and a caption line in the card's padding), and a story row per story, or per row of
    /// columns. Rows in view run, so the guess only places the rows past them until those scroll in.
    fn estimate(&self, tokens: &Tokens) -> f32 {
        let type_ = &tokens.typography;
        let header = type_.get(TypeRole::Title).line_height + type_.get(TypeRole::Caption).line_height + 2.0 * tokens.space.pad.regular;
        let row = tokens.size.control.md * STORY_ROW_IN_CONTROLS;
        let per_row = match self.layout {
            StoryLayout::Columns(most) => usize::from(most.max(1)),
            _ => 1,
        };
        header + self.stories.len().div_ceil(per_row) as f32 * row
    }

    /// The stories as the card takes them.
    pub fn card_stories(&self) -> Vec<CardStory<'_>> {
        self.stories.iter().map(|story| CardStory::new(story.index, story.story, &story.caption)).collect()
    }

    /// The card of this spec, with `stories` from [`Spec::card_stories`], `key` as its id scope and `room` as the
    /// most a filling story takes.
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

/// What a centre tab's view reads and edits besides its own values: the browser's state its cards show and change, and
/// where its requests go.
#[derive(MemoProps)]
#[non_exhaustive]
pub struct Centre<'a> {
    /// What is listed.
    pub catalog: &'a Catalog,
    /// The selected row of the tree; a card press selects its component.
    pub selection: &'a mut Tracked<Selection<TreeKey>>,
    /// The story each component's card shows.
    pub story_of: &'a mut Tracked<StoryOf>,
    /// The shown stories' states.
    pub cells: &'a mut Cells,
    /// The reset count the cards' id scopes carry.
    pub generation: u32,
    /// How the tab's selected card is inspected, while the inspector is on and the tab is in front.
    pub inspected: Option<Inspected>,
    /// The Variants view open in this tab.
    pub variants: Option<VariantsIn>,
    /// The components whose tab a card asked to open.
    pub opened: Outbox<'a, &'static str>,
    /// What the Variants view asked for.
    pub variants_out: Outbox<'a, VariantsEvent>,
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

/// The cards in one or two columns, scrolling vertically; each row is as tall as its tallest card. A row out of view
/// does not run (`Lazy`). Each card and each story in it is a memoized view that replays its last run until its inputs
/// change (the story's data by its revision, the card's look and state, the room and place). The card of the component
/// `reveal` names glides to the top of the view. The selected card is inspected as `inspected` says. The cards report
/// into `reported`.
#[expect(clippy::too_many_arguments, reason = "the grid's own values beside the shared cells; a struct would only rename them")]
pub(super) fn grid(
    ui: &mut Ui,
    scroll_id: impl std::hash::Hash + std::fmt::Debug,
    inspected: Option<&Inspected>,
    cards: &[Spec],
    cells: &mut Cells,
    generation: u32,
    one_column: bool,
    reveal: Option<&'static str>,
) -> Vec<Reported> {
    let mut reported = Vec::new();
    let tokens = Env::of(ui).tokens.clone();
    let columns = if one_column || ui.available_width() < 2.0 * card_min_width(&tokens) { 1 } else { 2 };
    // A story that fills takes its content height, at most the view's, as on the old gallery's tiles.
    let room = ui.available_height();
    Stack::new().fill().scroll(Scroll::vertical()).gap(Gap::CardRow).id_salt(scroll_id).show(ui, |ui, _| {
        reported.clear();
        for (row, chunk) in cards.chunks(columns).enumerate() {
            // A row out of view does not run. Keyed by its cards: a change of the column count regroups the rows into
            // new ones, which take their estimate until they run.
            let keys: Vec<&str> = chunk.iter().map(|spec| spec.key.as_str()).collect();
            let estimate = chunk.iter().map(|spec| spec.estimate(&tokens)).fold(0.0, f32::max);
            let revealed = chunk.iter().any(|spec| reveal == Some(spec.type_name));
            Lazy::new((generation, columns, keys), estimate).reveal(revealed).show(ui, |ui| {
                Grid::new(if columns == 1 { ONE } else { TWO }).gap(Gap::CardColumn).fill_width().show(ui, |ui, _| {
                    for (at, spec) in chunk.iter().enumerate() {
                        // A card out of view does not run and reports nothing, as it took no press and ran no command.
                        Lazy::new((generation, spec.key.as_str()), spec.estimate(&tokens)).show(ui, |ui| {
                            let stories = spec.card_stories();
                            let mut card = spec.card(&stories, format!("{}-{generation}", spec.key), room);
                            card.inspect = inspected.filter(|_| spec.selected).cloned();
                            let events = card.show(ui, cells);
                            reported.push(Reported { spec: row * columns + at, events });
                        });
                    }
                });
            });
        }
    });
    reported
}

/// Applies what the cards reported: a press selects the card's component and picks the card's first story unless the
/// picked one is on the card, Open asks for its tab, Reset gives the card's stories their start values. A value is
/// written only when it changes.
pub(super) fn apply(c: &mut Centre<'_>, cards: &[Spec], reported: Vec<Reported>) {
    for Reported { spec, events } in reported {
        let Some(spec) = cards.get(spec) else { continue };
        if events.reset {
            for story in &spec.stories {
                c.cells.state_of(story.index).reset();
            }
        }
        if events.opened {
            c.opened.send(spec.type_name);
        }
        if events.pressed || events.opened {
            let single = Selection::single(TreeKey::Component(spec.type_name));
            if **c.selection != single {
                **c.selection = single;
            }
            // The picked story stays while it is one of the card's; else the card's first story is picked.
            let picked = target_story(c.catalog, c.story_of, spec.type_name);
            if let Some(first) = spec.stories.first()
                && !spec.stories.iter().any(|story| Some(story.index) == picked)
            {
                c.story_of.insert(spec.type_name, first.story.story.name);
            }
        }
    }
}

/// What the Overview reads and edits.
#[derive(MemoProps)]
struct OverviewProps<'a> {
    centre: Centre<'a>,
    /// The text that narrows the levels' components.
    filter: &'a Tracked<String>,
    /// The level shown, by its title.
    level: &'a mut Tracked<String>,
    /// The selection the cards last followed.
    followed: &'a mut Tracked<Option<Option<&'static str>>>,
}

/// The Overview: the level select over the cards of the level's components.
pub(super) fn overview(
    ui: &mut Ui,
    centre: Centre<'_>,
    filter: &Tracked<String>,
    level: &mut Tracked<String>,
    followed: &mut Tracked<Option<Option<&'static str>>>,
) {
    let props = OverviewProps { centre, filter, level, followed };
    let _ = MemoView::new(view_id(&PanelId::Overview), props, overview_view).show(ui);
}

fn overview_view(p: &mut OverviewProps<'_>, ui: &mut Ui) {
    let OverviewProps { centre, filter, level: shown_level, followed } = p;
    let (shown_level, followed) = (&mut **shown_level, &mut **followed);
    if variants::show(ui, centre) {
        return;
    }
    // The selection, when it changed since the cards last followed it; its component's level becomes the shown one.
    let selected = selected_of(centre.selection);
    let follow = (**followed != Some(selected)).then_some(selected);
    if follow.is_some()
        && let Some(entry) = selected.and_then(entry_of)
        && shown_level.as_str() != entry.level.title()
    {
        **shown_level = entry.level.title().to_owned();
    }
    let groups = by_level(centre.catalog, filter.as_str());
    let Some(shown) = groups.iter().position(|(level, _)| level.title() == shown_level.as_str()).or(if groups.is_empty() { None } else { Some(0) })
    else {
        on_background(ui, "overview", |ui, _| {
            Notice::new(Tone::Info, "No components match the filter.").show(ui);
        });
        return;
    };
    let level = groups[shown].0;
    let entries: Vec<Entry<craft_ui::contract::Level>> = groups.iter().map(|(level, _)| Entry::new(*level, level.title())).collect();
    let cards: Vec<Spec> =
        groups[shown].1.iter().filter_map(|component| component_card(centre.catalog, centre.story_of, component, selected)).collect();
    let mut chosen = level;
    let mut reported = Vec::new();
    // The cards follow the selection in the pass they run: its card glides into view, and only then is it followed.
    let mut ran = false;
    let (inspected, generation) = (centre.inspected.clone(), centre.generation);
    on_background(ui, "overview", |ui, _| {
        ran = true;
        Row::new().show(ui, |ui, _| {
            Text::new("Level:").shade(Shade::Muted).show(ui);
            Select::new("Level", Bind::bound(&mut chosen), &entries).sizing(Sizing::Content).show(ui);
        });
        reported = grid(ui, level.title(), inspected.as_ref(), &cards, centre.cells, generation, false, follow.flatten());
    });
    if ran && let Some(selected) = follow {
        **followed = Some(selected);
    }
    if chosen != level {
        **shown_level = chosen.title().to_owned();
    }
    apply(centre, &cards, reported);
}

/// The stories of group `group` of the component registered as `type_name`, under their captions.
fn group_stories(catalog: &Catalog, type_name: &'static str, group: usize, name: &str) -> Vec<SpecStory> {
    catalog.group_stories(type_name, group).into_iter().map(|at| SpecStory::of(catalog, at, name)).collect()
}

/// The group the component registered as `type_name` shows: the group of its picked story, else its first.
fn target_group(catalog: &Catalog, story_of: &StoryOf, type_name: &'static str) -> usize {
    target_story(catalog, story_of, type_name).and_then(|at| catalog.group_at(type_name, at)).unwrap_or(0)
}

/// The Overview's card of `component`: the cases of the group of its picked story; selected when it is the selected
/// component.
fn component_card(catalog: &Catalog, story_of: &StoryOf, component: &ComponentInfo, selected: Option<&'static str>) -> Option<Spec> {
    let mut spec = overview_card(catalog, component, target_group(catalog, story_of, component.type_name))?;
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

/// What a component's tab reads and edits.
#[derive(MemoProps)]
struct ComponentTabProps<'a> {
    centre: Centre<'a>,
    /// The tab.
    tab: &'a PanelId,
    /// The registered name of the tab's component, when the catalog lists it.
    type_name: Option<&'static str>,
}

/// A component's tab: each story group of the component as a card in one column, titled with the group's name.
pub(super) fn component_tab(ui: &mut Ui, centre: Centre<'_>, tab: &PanelId) {
    let type_name = match tab {
        PanelId::Component(name) => centre.catalog.component_named(name),
        _ => None,
    };
    let props = ComponentTabProps { centre, tab, type_name };
    let _ = MemoView::new(view_id(tab), props, component_tab_view).show(ui);
}

fn component_tab_view(p: &mut ComponentTabProps<'_>, ui: &mut Ui) {
    let ComponentTabProps { centre, type_name, .. } = p;
    if variants::show(ui, centre) {
        return;
    }
    let found = type_name.and_then(|type_name| Some((type_name, entry_of(type_name)?, centre.catalog.component(type_name)?)));
    let Some((type_name, entry, component)) = found else {
        on_background(ui, "component", |ui, _| {
            Notice::new(Tone::Info, "The component has no stories.").show(ui);
        });
        return;
    };
    let target = target_group(centre.catalog, centre.story_of, type_name);
    let chosen = selected_of(centre.selection) == Some(type_name);
    let cards: Vec<Spec> = component
        .groups
        .iter()
        .enumerate()
        .map(|(at, group)| Spec {
            key: format!("{type_name}/{}", group.name),
            type_name,
            title: group.name.clone(),
            note: None,
            stories: group_stories(centre.catalog, type_name, at, &entry.name),
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
    let (inspected, generation) = (centre.inspected.clone(), centre.generation);
    on_background(ui, "component", |ui, _| {
        reported = grid(ui, ("component", type_name), inspected.as_ref(), &cards, centre.cells, generation, true, None);
    });
    apply(centre, &cards, reported);
}
