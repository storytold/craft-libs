//! The app shell's named slots: a slot keeps its region name on either edge.

use craft_ui::contract::{PaneEdge, Role, Show as _};
use craft_ui::display::Text;
use craft_ui::runtime::env::WindowKey;
use craft_ui::shell::AppShell;
use craft_ui::testing::{Frames, trial};

/// A tool rail placed at the end edge sits at the window's end and screen readers still find it as the Tools region.
#[trial]
fn the_tool_rail_at_the_end_edge_is_the_tools_region() {
    let mut frames = Frames::new();
    let mut placed = None;
    for _ in 0..3 {
        frames.frame(vec![], |ui| {
            let window = ui.max_rect();
            let out = AppShell::new(WindowKey::main())
                .tools(PaneEdge::End, true, |ui, _, _: &mut ()| {
                    Text::new("Rail").show(ui);
                })
                .body(|_, _, _| {})
                .show(ui, &mut ());
            placed = out.regions.slots.first().map(|(_, rect)| (*rect, window));
        });
    }
    let (rail, window) = placed.expect("the rail shows");
    assert!(rail.center().x > window.center().x, "the rail sits at the end edge: {rail:?} in {window:?}");
    let tree = frames.tree();
    let names: Vec<&str> = tree.with_role(Role::GROUP).into_iter().filter_map(|key| tree.name_of(key)).collect();
    assert!(names.contains(&"Tools"), "the end-edge rail is the Tools region: {names:?}");
}
