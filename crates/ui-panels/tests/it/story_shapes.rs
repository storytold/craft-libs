//! A fixture returns the clipped shapes of a frame that uploads textures.

use craft_ui::testing::{Fixture, Registry, trial};

/// Menu, rail and tile stories; the first frame of each uploads textures, the font atlas among them.
const STORIES: [&str; 6] =
    ["tool_rail_marked_in_a_closed_group", "edit_menu", "menu", "menu_taller_than_the_window", "menu_with_companion", "tile_actions"];

#[trial]
fn the_first_frame_of_a_story_returns_its_clipped_shapes() {
    let registry = Registry::of(&[craft_ui::testing::register, craft_ui_widgets::testing::register]);
    for name in STORIES {
        let entry = registry.stories().iter().find(|entry| entry.story.name == name).unwrap_or_else(|| panic!("story {name}"));
        let shapes = Fixture::story(&entry.story).frame_clipped_shapes();
        assert!(!shapes.is_empty(), "{name}: the frame paints the story");
    }
}
