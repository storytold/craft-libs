//! The viewer's own logic: default and saved layouts, and the crates that register components.

use craft_ui::testing::trial;
use craft_ui_gallery::viewer::{PanelId, Workspace, default_for, load_layout, open_tabs, save_layout};

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
            assert_eq!(layout.layout.contains(panel), panels.contains(panel), "{workspace:?} {panel:?}");
        }
    }
}

/// Regression: a saved layout naming a component the catalog lacks, or lacking a default panel, must still load.
#[trial]
fn a_saved_layout_loads_without_the_components_the_catalog_lacks_and_with_every_default_panel() {
    let mut saved = default_for(Workspace::Components);
    let _ = saved.set_visible(PanelId::Component("craft_ui::buttons::link::Link"), true, Some(PanelId::Overview), false, |panel| panel.permissions());
    let _ = saved.set_visible(PanelId::Component("gone::Removed"), true, Some(PanelId::Overview), false, |panel| panel.permissions());
    let _ = saved.set_visible(PanelId::Stories, false, None, false, |_| craft_behavior::dock::Permissions::ALL);
    assert!(!saved.layout.contains(&PanelId::Stories));
    let loaded = load_layout(&save_layout(&saved), Workspace::Components, &|name| name == "craft_ui::buttons::link::Link");
    assert_eq!(loaded.validate(), Ok(()));
    assert_eq!(open_tabs(&loaded), ["craft_ui::buttons::link::Link"]);
    assert!(loaded.hidden.iter().all(|(panel, _)| !matches!(panel, PanelId::Component(_))));
    for panel in COMPONENTS {
        assert!(loaded.layout.contains(&panel), "{panel:?}");
    }
    let unreadable = load_layout("{\"layout\":", Workspace::Theme, &|_| true);
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
