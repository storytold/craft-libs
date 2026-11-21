//! The viewer's own logic: default and saved layouts, and the crates that register components.

use craft_ui::contract::Level;
use craft_ui::testing::trial;
use craft_ui_gallery::viewer::{Catalog, ComponentInfo, PanelId, TabName, Workspace, default_for, load_layout, open_tabs, save_layout};

const COMPONENTS: [PanelId; 6] = [PanelId::Components, PanelId::Stories, PanelId::Properties, PanelId::Inspect, PanelId::Overview, PanelId::Tokens];
const THEME: [PanelId; 5] = [PanelId::Preview, PanelId::Detail, PanelId::Problems, PanelId::AppSizes, PanelId::Tokens];

/// Regression: a workspace's default layout that held another workspace's panel, or lost one of its own, opened the
/// gallery with a missing or foreign panel.
#[trial]
fn each_default_layout_is_valid_and_holds_exactly_its_workspaces_panels() {
    for (workspace, panels) in [(Workspace::Components, &COMPONENTS[..]), (Workspace::Theme, &THEME[..])] {
        let layout = default_for(workspace);
        assert_eq!(layout.validate(), Ok(()));
        for panel in COMPONENTS.iter().chain(&THEME) {
            assert_eq!(layout.layout().contains(panel), panels.contains(panel), "{workspace:?} {panel:?}");
        }
    }
}

/// A saved layout keeps its component tabs by name across a save and a load; a tab the catalog does not list, and a
/// default panel the save lacks, come back as the catalog and the default have them.
#[trial]
fn a_saved_layout_round_trips_its_component_tabs_by_name() {
    const LINK: &str = "craft_ui::buttons::link::Link";
    let catalog = Catalog::listing(vec![ComponentInfo::new(LINK, Level::Controls, "Link", &[])]);
    let open = |names: &[&str]| {
        let mut dock = default_for(Workspace::Components);
        for name in names {
            let _ = dock.set_visible(PanelId::component(name), true, Some(PanelId::Overview), false, |panel| panel.permissions());
        }
        dock
    };
    let mut saved = open(&[LINK, "gone::Removed"]);
    let _ = saved.set_visible(PanelId::Stories, false, None, false, |_| craft_behavior::dock::Permissions::ALL);
    let loaded = load_layout(&save_layout(&saved), Workspace::Components, &catalog);
    assert_eq!(loaded, open(&[LINK]));
    assert_eq!(open_tabs(&loaded), [TabName::from(LINK)]);
    let unreadable = load_layout("{\"layout\":", Workspace::Theme, &catalog);
    assert_eq!(unreadable, default_for(Workspace::Theme));
}

/// A crate with no registrations here was linked out, which in a release web build loses its stories.
#[trial]
fn every_linked_crate_registers() {
    let (counts, stories) = craft_ui_gallery::linked_counts();
    assert!(stories > 0);
    let empty: Vec<_> = counts.iter().filter(|(_, count)| *count == 0).collect();
    assert!(empty.is_empty(), "no registrations from {empty:?}");
}

/// The Overview stays docked and open; a component tab beside it still floats and closes.
#[trial]
fn the_overview_is_pinned_and_a_component_tab_is_not() {
    use craft_behavior::dock::{Action, DockPlacement};
    let rect = egui::Rect::from_min_size(egui::pos2(40.0, 40.0), egui::vec2(300.0, 200.0));
    let tab = PanelId::component("craft_ui::buttons::link::Link");
    let mut dock = default_for(Workspace::Components);
    let _ = dock.set_visible(tab.clone(), true, Some(PanelId::Overview), false, |panel| panel.permissions());
    assert!(dock.layout().contains(&tab), "the component tab opens beside the Overview");
    let refused = [
        Action::float(PanelId::Overview, rect),
        Action::close(PanelId::Overview),
        Action::move_beside(PanelId::Overview, PanelId::Properties, DockPlacement::tab(None)),
    ];
    for action in refused {
        let mut tried = dock.clone();
        assert!(tried.apply(action.clone(), false, |panel| panel.permissions()).is_err(), "refused: {action:?}");
    }
    let mut floated = dock.clone();
    assert!(floated.apply(Action::float(tab.clone(), rect), false, |panel| panel.permissions()).is_ok(), "the component tab floats");
    assert!(dock.apply(Action::close(tab.clone()), false, |panel| panel.permissions()).is_ok(), "the component tab closes");
}
