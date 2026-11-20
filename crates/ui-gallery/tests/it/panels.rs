//! The viewer's panels as memoized views: which panel bodies run for an interaction, counted from the memo log.

use craft_behavior::Axis;
use craft_behavior::dock::{Column, DockEdge, Group, Layout, Node, SplitSize, Workspace as Dock};
use craft_settings::{Location, OpenOptions, SettingsStore};
use craft_ui::runtime::access::Role;
use craft_ui::shell::app::ShellApp;
use craft_ui::testing::{Fixture, MemoRun, input, last_frame_memo_runs, pointer, trial};
use craft_ui_gallery::chrome::{Gallery, Persisted};
use craft_ui_gallery::viewer::{Catalog, GallerySettings, PanelId, default_components, save_layout};
use egui::{Event, Vec2};
use std::collections::BTreeSet;

/// The panel bodies by their props types.
const PANELS: [(&str, &str); 6] = [
    ("::ComponentsProps<", "Components"),
    ("::StoriesProps<", "Stories"),
    ("::PropertiesProps<", "Properties"),
    ("::InspectProps<", "Inspect"),
    ("::OverviewProps<", "Overview"),
    ("::ComponentTabProps<", "Component tab"),
];

/// The components workspace with the tree, the Overview, and the stories, properties and inspector each in a group of
/// its own, so all their bodies show at once.
fn side_by_side() -> Dock<PanelId> {
    let tree = Column::new(vec![Group::new(vec![PanelId::Components], 0)]).with_width(220.0);
    let side = Column::new(vec![
        Group::new(vec![PanelId::Stories], 0),
        Group::new(vec![PanelId::Properties], 0),
        Group::new(vec![PanelId::Inspect, PanelId::Tokens], 0),
    ]);
    let half = SplitSize::Ratio(0.5);
    let centre = Node::split(Axis::X, half, Node::tabs(vec![PanelId::Overview], 0), Node::dock(DockEdge::End, vec![side]));
    Dock::new(Layout::new(Some(Node::split(Axis::X, half, Node::dock(DockEdge::Start, vec![tree]), centre)), Vec::new()))
}

/// The gallery on the Controls level narrowed by `filter`, with the component shown as `selected` selected (none when
/// empty), in the dock layout `dock`, settled in a 1400 by 900 window.
fn fixture(filter: &str, selected: &str, dock: &Dock<PanelId>) -> Fixture {
    let (mut store, report) = SettingsStore::<GallerySettings>::open_with(OpenOptions::new(Location::Memory));
    let mut settings = store.get().clone();
    settings.level = "Controls".to_owned();
    settings.filter = filter.to_owned();
    settings.dock = save_layout(dock);
    settings.selected = Catalog::linked()
        .components
        .iter()
        .find(|component| component.name == selected)
        .map(|component| component.type_name.to_owned())
        .unwrap_or_default();
    store.set(settings);
    let mut gallery = Gallery::new(Catalog::linked(), Persisted::new(store, &report));
    let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
    fixture.room(Vec2::new(1400.0, 900.0)).settle().idle(3);
    fixture
}

/// The panel bodies of the last frame with how each took part.
fn panel_runs(fixture: &Fixture) -> Vec<(&'static str, MemoRun)> {
    last_frame_memo_runs(fixture.ctx())
        .into_iter()
        .filter_map(|(ty, _, run)| PANELS.iter().find(|(name, _)| ty.contains(name)).map(|(_, name)| (*name, run)))
        .collect()
}

/// The panel bodies that ran in the last frame.
fn ran(fixture: &Fixture) -> BTreeSet<&'static str> {
    panel_runs(fixture).into_iter().filter(|(_, run)| *run == MemoRun::Ran).map(|(name, _)| name).collect()
}

/// Plays `frames`, then settles, and returns every panel body that ran on the way; then checks the next idle frame runs
/// none but the ones in `focused`, which run while focus is in them.
fn ran_over(fixture: &mut Fixture, frames: Vec<Vec<Event>>, focused: &[&'static str]) -> BTreeSet<&'static str> {
    let mut all = BTreeSet::new();
    for events in frames {
        fixture.frame(events);
        all.extend(ran(fixture));
    }
    // Until the change has settled: a selected card glides into view.
    for _ in 0..64 {
        fixture.frame(Vec::new());
        let now = ran(fixture);
        all.extend(now.iter().copied());
        if now.is_subset(&names(focused)) && !fixture.repaint_requested() {
            break;
        }
    }
    fixture.idle(1);
    assert!(ran(fixture).is_subset(&names(focused)), "after the change every panel body replays: {:?}", ran(fixture));
    all
}

fn names(list: &[&'static str]) -> BTreeSet<&'static str> {
    list.iter().copied().collect()
}

/// An idle viewer runs no panel body: the tree, the stories, the properties, the inspector and the Overview replay.
#[trial]
fn an_idle_viewer_runs_no_panel_body() {
    let fixture = fixture("Slider", "", &side_by_side());
    let replayed: BTreeSet<_> = panel_runs(&fixture).into_iter().filter(|(_, run)| *run == MemoRun::Replayed).map(|(name, _)| name).collect();
    assert_eq!(replayed, names(&["Components", "Stories", "Properties", "Inspect", "Overview"]));
    assert!(ran(&fixture).is_empty(), "an idle frame ran {:?}", ran(&fixture));
}

/// A filter edit runs the tree and the Overview, which lists the filtered level, and no other panel body. The tree keeps
/// running while focus stays in the filter field.
#[trial]
fn a_filter_edit_runs_the_tree_and_the_overview_only() {
    let mut fixture = fixture("Slider", "", &side_by_side());
    let field = fixture.by_role(Role::TEXT_INPUT).into_iter().next().expect("the filter field").center();
    let ran = ran_over(
        &mut fixture,
        vec![vec![pointer::moved(field)], vec![pointer::press(field)], vec![pointer::release(field)], vec![input::text("x")]],
        &["Components"],
    );
    assert_eq!(ran, names(&["Components", "Overview"]));
}

/// A selection change runs the panel bodies that read the selection: the tree, the stories, the properties, the
/// inspector and the Overview. The tree keeps running while focus stays in it.
#[trial]
fn a_selection_change_runs_only_the_panels_that_read_it() {
    let mut fixture = fixture("Slider", "", &side_by_side());
    let row = fixture
        .by_role(Role::TREE_ITEM)
        .into_iter()
        .find(|row| fixture.tree().name_of(row.node).is_some_and(|name| name != "Controls"))
        .expect("a component row")
        .center();
    let ran = ran_over(&mut fixture, vec![vec![pointer::moved(row)], vec![pointer::press(row)], vec![pointer::release(row)]], &["Components"]);
    assert_eq!(ran, names(&["Components", "Stories", "Properties", "Inspect", "Overview"]));
}

/// A value edit in Properties runs Properties, the Overview that hosts the story's card, and that one card, in the
/// frame of the edit: the Overview's other cards replay, and no other panel body runs.
#[trial]
fn a_story_value_edit_runs_properties_and_its_card() {
    let mut fixture = fixture("Checkbox", "Checkbox", &default_components());
    // The rightmost checkbox is an editor in Properties, at the end edge.
    let editor =
        fixture.by_role(Role::CHECK_BOX).into_iter().max_by(|a, b| a.center().x.total_cmp(&b.center().x)).expect("a checkbox in Properties").center();
    let mut panels = BTreeSet::new();
    let mut cards = BTreeSet::new();
    // The frames in which a card ran: the release that toggles the editor is frame 2.
    let mut card_frames = Vec::new();
    for (at, events) in
        [vec![pointer::moved(editor)], vec![pointer::press(editor)], vec![pointer::release(editor)], vec![pointer::moved(egui::pos2(1.0, 1.0))]]
            .into_iter()
            .enumerate()
    {
        fixture.frame(events);
        panels.extend(ran(&fixture));
        if !cards_ran(&fixture).is_empty() {
            card_frames.push(at);
        }
        cards.extend(cards_ran(&fixture));
    }
    for at in 4..68 {
        fixture.frame(Vec::new());
        let now = ran(&fixture);
        panels.extend(now.iter().copied());
        if !cards_ran(&fixture).is_empty() {
            card_frames.push(at);
        }
        cards.extend(cards_ran(&fixture));
        if now.is_empty() && !fixture.repaint_requested() {
            break;
        }
    }
    assert_eq!(panels, names(&["Properties", "Overview"]));
    assert_eq!(cards.len(), 1, "one card ran: {cards:?}");
    // The story's checkbox animates its new state after that, which runs the card in the next frames.
    assert_eq!(card_frames.first(), Some(&2), "the card shows the edit in the frame of the edit: {card_frames:?}");
    fixture.idle(1);
    assert!(ran(&fixture).is_empty() && cards_ran(&fixture).is_empty(), "then everything replays: {:?}", ran(&fixture));
}

/// The card views that ran in the last frame, by id.
fn cards_ran(fixture: &Fixture) -> BTreeSet<u64> {
    last_frame_memo_runs(fixture.ctx())
        .into_iter()
        .filter(|(ty, _, run)| ty.contains("::CardProps<") && *run == MemoRun::Ran)
        .map(|(_, id, _)| id.value())
        .collect()
}
